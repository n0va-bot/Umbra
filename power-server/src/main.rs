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

unsafe fn sys_outw(port: u16, val: u16) {
    syscall(14, port as u64, val as u64, 0, 0, 0);
}

fn ipc_recv(endpoint: usize, msg: &mut Message) -> Result<(), ()> {
    let result = unsafe { syscall(101, endpoint as u64, msg as *mut Message as u64, 0, 0, 0) };
    if result == 0 { Ok(()) } else { Err(()) }
}

fn sys_claim_endpoint(endpoint: usize) -> Result<(), ()> {
    let result = unsafe { syscall(103, endpoint as u64, 0, 0, 0, 0) };
    if result == 0 { Ok(()) } else { Err(()) }
}

const POWER_SERVER: usize = 14;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    if sys_claim_endpoint(POWER_SERVER).is_err() {
        loop {
            unsafe { syscall(8, 0, 0, 0, 0, 0) };
        }
    }

    loop {
        let mut msg = Message::empty();
        if ipc_recv(POWER_SERVER, &mut msg).is_ok() {
            if msg.tag == 1 {
                // POWER_OFF
                unsafe { sys_outw(0xb004, 0x2000) };
                
                // QEMU ACPI PM1a control port is typically 0x604 on Q35
                // We hardcode it here until we port full ACPI parsing to userspace
                unsafe { sys_outw(0x604, 0x2000) };

                // Fallback for QEMU isa-debug-exit
                unsafe { sys_outw(0x501, 0) };
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
