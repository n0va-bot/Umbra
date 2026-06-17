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

fn sys_inb(port: u16) -> u8 {
    unsafe { syscall(11, port as u64, 0, 0, 0, 0) as u8 }
}

fn sys_irq_subscribe(irq: u8, endpoint: usize) -> bool {
    let res = unsafe { syscall(18, irq as u64, endpoint as u64, 0, 0, 0) };
    res == 0
}

const KB_GET_CHAR: u32 = 2;
const KB_GET_CHAR_POLL: u32 = 3;
const IRQ_MESSAGE_TAG: u32 = 0xFFFFFFFF;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let my_endpoint = create_endpoint().expect("keyboard-server: failed to create endpoint");

    // Subscribe to IRQ 1
    if !sys_irq_subscribe(1, my_endpoint) {
        // We'll panic below if it fails, or just spin
    }

    // Register with SerV
    let mut reg_msg = Message::empty();
    reg_msg.tag = 1;
    reg_msg.data[0..8].copy_from_slice(b"keyboard");
    reg_msg.data[8..16].copy_from_slice(&(my_endpoint as u64).to_le_bytes());
    let _ = ipc_send(1, &reg_msg);

    let mut keyboard = pc_keyboard::Keyboard::new(
        pc_keyboard::ScancodeSet1::new(),
        pc_keyboard::layouts::Us104Key,
        pc_keyboard::HandleControl::Ignore,
    );

    let mut char_buffer = [0u32; 128];
    let mut read_idx = 0;
    let mut write_idx = 0;
    let mut pending_reply: Option<(usize, u32)> = None;

    loop {
        let mut msg = Message::empty();
        if ipc_recv(my_endpoint, &mut msg).is_ok() {
            if msg.tag == IRQ_MESSAGE_TAG {
                // IRQ fired, read port 0x64 until empty
                while sys_inb(0x64) & 1 == 1 {
                    let scancode = sys_inb(0x60);
                    if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
                        if let Some(key) = keyboard.process_keyevent(key_event) {
                            let mut char_code = 0u32;
                            match key {
                                pc_keyboard::DecodedKey::Unicode(c) => char_code = c as u32,
                                pc_keyboard::DecodedKey::RawKey(k) => {
                                    if k == pc_keyboard::KeyCode::Backspace {
                                        char_code = 8;
                                    }
                                }
                            }

                            if char_code != 0 {
                                if let Some((endpoint, reply_tag)) = pending_reply.take() {
                                    let mut reply = Message::empty();
                                    reply.tag = reply_tag;
                                    reply.data[0..4].copy_from_slice(&char_code.to_le_bytes());
                                    let _ = ipc_send(endpoint, &reply);
                                } else {
                                    char_buffer[write_idx] = char_code;
                                    write_idx = (write_idx + 1) % 128;
                                }
                            }
                        }
                    }
                }
            } else if msg.tag == KB_GET_CHAR || msg.tag == KB_GET_CHAR_POLL {
                let reply_endpoint = usize::from_le_bytes(msg.data[0..8].try_into().unwrap());

                if read_idx != write_idx {
                    let c = char_buffer[read_idx];
                    read_idx = (read_idx + 1) % 128;

                    let mut reply = Message::empty();
                    reply.tag = msg.tag;
                    reply.data[0..4].copy_from_slice(&c.to_le_bytes());
                    let _ = ipc_send(reply_endpoint, &reply);
                } else if msg.tag == KB_GET_CHAR_POLL {
                    let mut reply = Message::empty();
                    reply.tag = KB_GET_CHAR_POLL;
                    reply.data[0..4].copy_from_slice(&(u32::MAX).to_le_bytes());
                    let _ = ipc_send(reply_endpoint, &reply);
                } else {
                    pending_reply = Some((reply_endpoint, msg.tag));
                }
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
