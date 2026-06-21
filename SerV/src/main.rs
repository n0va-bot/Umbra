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

const SYS_IPC_SEND: u64 = 100;
const SYS_IPC_RECV: u64 = 101;
const SYS_IPC_CLAIM_ENDPOINT: u64 = 103;
const SYS_SPAWN: u64 = 105;

#[inline(always)]
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

fn claim_endpoint(endpoint: usize) -> bool {
    let result = unsafe { syscall(SYS_IPC_CLAIM_ENDPOINT, endpoint as u64, 0, 0, 0, 0) };
    result == 0
}

fn spawn(name: &str) -> u64 {
    unsafe { syscall(SYS_SPAWN, name.as_ptr() as u64, name.len() as u64, 0, 0, 0) }
}

fn grant_cap(pid: u64, cap_type: u8, cap_arg: u16, arg_extra: u64) -> bool {
    unsafe { syscall(19, pid, cap_type as u64, cap_arg as u64, arg_extra, 0) == 0 }
}

fn ipc_recv(endpoint: usize, msg: &mut Message) -> Result<(), ()> {
    let result = unsafe {
        syscall(
            SYS_IPC_RECV,
            endpoint as u64,
            msg as *mut Message as u64,
            0,
            0,
            0,
        )
    };
    if result == 0 { Ok(()) } else { Err(()) }
}

fn ipc_send(endpoint: usize, msg: &Message) -> Result<(), ()> {
    let result = unsafe {
        syscall(
            SYS_IPC_SEND,
            endpoint as u64,
            msg as *const Message as u64,
            0,
            0,
            0,
        )
    };
    if result == 0 { Ok(()) } else { Err(()) }
}

const FB_SERVER: usize = 11;
const FB_WRITE_CHAR: u32 = 1;

struct Stdout;
impl core::fmt::Write for Stdout {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for byte in s.bytes() {
            unsafe { syscall(0, byte as u64, 0, 0, 0, 0) };
        }
        Ok(())
    }
}
macro_rules! print {
    ($($arg:tt)*) => {
        let _ = core::fmt::Write::write_fmt(&mut Stdout, format_args!($($arg)*));
    };
}
macro_rules! println {
    () => (print!("\n"));
    ($($arg:tt)*) => (print!("{}\n", format_args!($($arg)*)));
}

#[derive(Clone, Copy)]
struct Service {
    name: [u8; 8],
    endpoint: usize,
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    if !claim_endpoint(1) {
        println!("[SerV] Failed to claim endpoint 1");
        loop {}
    }

    println!("[SerV] Claimed endpoint 1. Starting core stage...");

    let p_fb = spawn("fb-server");
    println!("[SerV] spawned fb-server, pid: {}", p_fb);
    grant_cap(p_fb, 2, 11, 0);
    grant_cap(p_fb, 3, 0, 0);

    let p1 = spawn("keyboard-server");
    println!("[SerV] spawned keyboard-server, pid: {}", p1);
    grant_cap(p1, 2, 1, 0);
    if !grant_cap(p1, 0, 0x60, 0) {
        println!("[SerV] failed to grant port 0x60");
    }
    if !grant_cap(p1, 0, 0x64, 0) {
        println!("[SerV] failed to grant port 0x64");
    }
    if !grant_cap(p1, 1, 1, 0) {
        println!("[SerV] failed to grant IRQ 1");
    }

    let p2 = spawn("tick-server");
    println!("[SerV] spawned tick-server, pid: {}", p2);
    grant_cap(p2, 2, 1, 0);
    grant_cap(p2, 2, 16, 0);

    let p4 = spawn("rtc-server");
    println!("[SerV] spawned rtc-server, pid: {}", p4);
    grant_cap(p4, 2, 12, 0);
    grant_cap(p4, 0, 0x70, 0);
    grant_cap(p4, 0, 0x71, 0);

    let p5 = spawn("pci-server");
    println!("[SerV] spawned pci-server, pid: {}", p5);
    grant_cap(p5, 2, 13, 0);
    grant_cap(p5, 0, 0xCF8, 0);
    grant_cap(p5, 0, 0xCFC, 0);

    let p6 = spawn("power-server");
    println!("[SerV] spawned power-server, pid: {}", p6);
    grant_cap(p6, 2, 14, 0);
    grant_cap(p6, 0, 0xb004, 0);
    grant_cap(p6, 0, 0x604, 0);
    grant_cap(p6, 0, 0x501, 0);

    let p3 = spawn("userspace");
    println!("[SerV] spawned userspace, pid: {}", p3);
    grant_cap(p3, 2, 1, 0);
    grant_cap(p3, 2, 11, 0);
    grant_cap(p3, 2, 12, 0);
    grant_cap(p3, 2, 13, 0);
    grant_cap(p3, 2, 14, 0);
    grant_cap(p3, 2, 16, 0);

    println!("[SerV] Entering service manager loop.");

    let mut services = [Service {
        name: [0; 8],
        endpoint: 0,
    }; 32];
    let mut num_services = 0;

    loop {
        let mut msg = Message::empty();
        if ipc_recv(1, &mut msg).is_ok() {
            if msg.tag == 1 {
                // REGISTER_SERVICE
                let endpoint = usize::from_le_bytes(msg.data[8..16].try_into().unwrap());
                let mut name = [0u8; 8];
                name.copy_from_slice(&msg.data[0..8]);

                if num_services < services.len() {
                    services[num_services] = Service { name, endpoint };
                    num_services += 1;
                    if let Ok(name_str) = core::str::from_utf8(&name) {
                        println!("[SerV] Registered '{}' at endpoint {}", name_str, endpoint);
                    }
                }
            } else if msg.tag == 2 {
                // LOOKUP_SERVICE
                let mut name = [0u8; 8];
                name.copy_from_slice(&msg.data[0..8]);
                let reply_endpoint = usize::from_le_bytes(msg.data[8..16].try_into().unwrap());

                let mut found_endpoint = usize::MAX;
                for i in 0..num_services {
                    if services[i].name == name {
                        found_endpoint = services[i].endpoint;
                        break;
                    }
                }

                let mut reply = Message::empty();
                reply.tag = 2; // LOOKUP_REPLY
                reply.data[0..8].copy_from_slice(&found_endpoint.to_le_bytes());
                let _ = ipc_send(reply_endpoint, &reply);
            }
        }
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("[SerV] PANIC: {}", info);
    loop {}
}
