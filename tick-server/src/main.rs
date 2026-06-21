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
const SYS_IPC_CALL: u64 = 102;
const SYS_IPC_CREATE_ENDPOINT: u64 = 104;

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

fn create_endpoint() -> Option<usize> {
    let result = unsafe { syscall(SYS_IPC_CREATE_ENDPOINT, 0, 0, 0, 0, 0) };
    if result == u64::MAX {
        None
    } else {
        Some(result as usize)
    }
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
    loop {
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
        if result == 0 {
            return Ok(());
        }
        if result == u64::MAX {
            return Err(());
        }
        unsafe { syscall(7, 0, 0, 0, 0, 0) };
    }
}

const RAW_TICK: usize = 16;
const TICK_GET: u32 = 1;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let my_endpoint = create_endpoint().expect("tick-server: failed to create endpoint");

    // Register with SerV
    let mut reg_msg = Message::empty();
    reg_msg.tag = 1;
    reg_msg.data[0..8].copy_from_slice(b"tick\0\0\0\0");
    reg_msg.data[8..16].copy_from_slice(&(my_endpoint as u64).to_le_bytes());
    let _ = ipc_send(1, &reg_msg);

    loop {
        let mut msg = Message::empty();
        if ipc_recv(my_endpoint, &mut msg).is_ok() {
            if msg.tag == TICK_GET {
                let reply_endpoint = usize::from_le_bytes(msg.data[0..8].try_into().unwrap());

                let req = Message::new(TICK_GET, &[]);
                let mut resp = Message::empty();
                if unsafe {
                    syscall(
                        SYS_IPC_CALL,
                        RAW_TICK as u64,
                        &req as *const _ as u64,
                        &mut resp as *mut _ as u64,
                        0,
                        0,
                    )
                } == 0
                {
                    let mut reply = Message::empty();
                    reply.tag = TICK_GET;
                    reply.data[0..8].copy_from_slice(&resp.data[0..8]);
                    let _ = ipc_send(reply_endpoint, &reply);
                }
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
