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

unsafe fn sys_inl(port: u16) -> u32 {
    syscall(15, port as u64, 0, 0, 0, 0) as u32
}

unsafe fn sys_outl(port: u16, val: u32) {
    syscall(16, port as u64, val as u64, 0, 0, 0);
}

fn ipc_recv(endpoint: usize, msg: &mut Message) -> Result<(), ()> {
    let result = unsafe { syscall(101, endpoint as u64, msg as *mut Message as u64, 0, 0, 0) };
    if result == 0 { Ok(()) } else { Err(()) }
}

fn ipc_send(endpoint: usize, msg: &Message) -> Result<(), ()> {
    loop {
        let result =
            unsafe { syscall(100, endpoint as u64, msg as *const Message as u64, 0, 0, 0) };
        if result == 0 {
            return Ok(());
        }
        if result == u64::MAX {
            return Err(());
        }
        // Queue full, yield and retry
        unsafe { syscall(7, 0, 0, 0, 0, 0) }; // sys_yield
    }
}

fn sys_claim_endpoint(endpoint: usize) -> Result<(), ()> {
    let result = unsafe { syscall(103, endpoint as u64, 0, 0, 0, 0) };
    if result == 0 { Ok(()) } else { Err(()) }
}

fn pci_config_address(bus: u8, device: u8, func: u8, offset: u8) -> u32 {
    1 << 31
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((func as u32) << 8)
        | (offset as u32 & 0xFC)
}

unsafe fn pci_read_u32(bus: u8, device: u8, func: u8, offset: u8) -> u32 {
    let address = pci_config_address(bus, device, func, offset);
    unsafe {
        sys_outl(0xCF8, address);
        sys_inl(0xCFC)
    }
}

const PCI_SERVER: usize = 13;
const FB_SERVER: usize = 11;

fn print_str(s: &str) {
    for c in s.bytes() {
        let msg = Message::new(1, &[c]);
        let _ = ipc_send(FB_SERVER, &msg);
    }
}

fn write_hex(mut val: u32, digits: u8) {
    let mut buf = [b'0'; 8];
    for i in 0..digits {
        let hex = (val & 0xF) as u8;
        buf[(digits - 1 - i) as usize] = if hex < 10 {
            b'0' + hex
        } else {
            b'A' + hex - 10
        };
        val >>= 4;
    }
    print_str(core::str::from_utf8(&buf[..digits as usize]).unwrap());
}

fn scan_buses(reply_endpoint: usize) {
    for bus in 0..=255 {
        for device in 0..=31 {
            for func in 0..=7 {
                unsafe {
                    let reg0 = pci_read_u32(bus, device, func, 0);
                    let vendor_id = (reg0 & 0xFFFF) as u16;

                    if vendor_id != 0xFFFF {
                        let device_id = (reg0 >> 16) as u16;

                        let reg8 = pci_read_u32(bus, device, func, 8);
                        let class_code = (reg8 >> 24) as u8;
                        let subclass = (reg8 >> 16) as u8;

                        let mut reply = Message::empty();
                        reply.tag = 1;
                        reply.data[0] = bus;
                        reply.data[1] = device;
                        reply.data[2] = func;
                        reply.data[3] = class_code;
                        reply.data[4] = subclass;
                        reply.data[5..7].copy_from_slice(&vendor_id.to_le_bytes());
                        reply.data[7..9].copy_from_slice(&device_id.to_le_bytes());

                        let _ = ipc_send(reply_endpoint, &reply);
                    }
                }
            }
        }
    }

    // Send "done" message
    let mut done_msg = Message::empty();
    done_msg.tag = 2;
    let _ = ipc_send(reply_endpoint, &done_msg);
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    if sys_claim_endpoint(PCI_SERVER).is_err() {
        loop {
            unsafe { syscall(8, 0, 0, 0, 0, 0) };
        }
    }

    loop {
        let mut msg = Message::empty();
        if ipc_recv(PCI_SERVER, &mut msg).is_ok() {
            if msg.tag == 1 {
                // PCI_SCAN_BUSES
                let reply_endpoint = usize::from_le_bytes(msg.data[0..8].try_into().unwrap());
                scan_buses(reply_endpoint);
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
