#![no_std]
#![no_main]
#![allow(unsafe_op_in_unsafe_fn)]

use core::arch::asm;
use core::panic::PanicInfo;

const IPC_MSG_DATA_SIZE: usize = 64;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Message {
    tag: u32,
    data: [u8; IPC_MSG_DATA_SIZE],
}

impl Message {
    fn new(tag: u32, data: &[u8]) -> Self {
        let mut msg = Self {
            tag,
            data: [0; IPC_MSG_DATA_SIZE],
        };
        let copy_len = data.len().min(IPC_MSG_DATA_SIZE);
        msg.data[..copy_len].copy_from_slice(&data[..copy_len]);
        msg
    }

    fn empty() -> Self {
        Self {
            tag: 0,
            data: [0; IPC_MSG_DATA_SIZE],
        }
    }
}

unsafe fn syscall(n: u64, arg1: u64, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> u64 {
    let ret: u64;
    asm!(
        "syscall",
        in("rax") n,
        in("rdi") arg1,
        in("rsi") arg2,
        in("rdx") arg3,
        in("r10") arg4,
        in("r8") arg5,
        out("rcx") _,
        out("r11") _,
        lateout("rax") ret,
        options(nostack, preserves_flags)
    );
    ret
}

unsafe fn sys_inb(port: u16) -> u8 {
    syscall(11, port as u64, 0, 0, 0, 0) as u8
}

unsafe fn sys_outb(port: u16, val: u8) {
    syscall(12, port as u64, val as u64, 0, 0, 0);
}

fn ipc_recv(endpoint: usize, msg: &mut Message) -> Result<(), ()> {
    let result = unsafe { syscall(101, endpoint as u64, msg as *mut Message as u64, 0, 0, 0) };
    if result == 0 { Ok(()) } else { Err(()) }
}

fn ipc_send(endpoint: usize, msg: &Message) -> Result<(), ()> {
    loop {
        let result = unsafe { syscall(100, endpoint as u64, msg as *const Message as u64, 0, 0, 0) };
        if result == 0 { return Ok(()); }
        if result == u64::MAX { return Err(()); }
        // Queue full, yield and retry
        unsafe { syscall(7, 0, 0, 0, 0, 0) }; // sys_yield
    }
}

fn sys_claim_endpoint(endpoint: usize) -> Result<(), ()> {
    let result = unsafe { syscall(103, endpoint as u64, 0, 0, 0, 0) };
    if result == 0 { Ok(()) } else { Err(()) }
}

struct Cmos {
    addr_port: u16,
    data_port: u16,
}

impl Cmos {
    fn new() -> Self {
        Self {
            addr_port: 0x70,
            data_port: 0x71,
        }
    }

    unsafe fn read_register(&mut self, reg: u8) -> u8 {
        unsafe {
            sys_outb(self.addr_port, reg);
            sys_inb(self.data_port)
        }
    }

    fn bcd_to_binary(bcd: u8) -> u8 {
        (bcd & 0x0F) + ((bcd / 16) * 10)
    }

    fn read_time(&mut self) -> (u8, u8, u8, u8, u8, u8) {
        unsafe {
            while self.read_register(0x0A) & 0x80 != 0 {
                core::hint::spin_loop();
            }

            let seconds = Self::bcd_to_binary(self.read_register(0x00));
            let minutes = Self::bcd_to_binary(self.read_register(0x02));
            let hours = Self::bcd_to_binary(self.read_register(0x04));
            let day = Self::bcd_to_binary(self.read_register(0x07));
            let month = Self::bcd_to_binary(self.read_register(0x08));
            let year = Self::bcd_to_binary(self.read_register(0x09));

            (year, month, day, hours, minutes, seconds)
        }
    }
}

const RTC_SERVER: usize = 12;
const FB_SERVER: usize = 11;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    if sys_claim_endpoint(RTC_SERVER).is_err() {
        loop {
            unsafe { syscall(8, 0, 0, 0, 0, 0) };
        }
    }

    let mut cmos = Cmos::new();

    loop {
        let mut msg = Message::empty();
        if ipc_recv(RTC_SERVER, &mut msg).is_ok() {
            if msg.tag == 1 {
                // RTC_GET_TIME
                let reply_endpoint = usize::from_le_bytes(msg.data[0..8].try_into().unwrap());
                let (year, month, day, hours, minutes, seconds) = cmos.read_time();

                let mut reply = Message::empty();
                reply.tag = 1;
                reply.data[0] = year;
                reply.data[1] = month;
                reply.data[2] = day;
                reply.data[3] = hours;
                reply.data[4] = minutes;
                reply.data[5] = seconds;

                let _ = ipc_send(reply_endpoint, &reply);
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
