#![no_std]
#![no_main]
#![allow(unsafe_op_in_unsafe_fn)]

use core::arch::asm;
use core::panic::PanicInfo;

// IPC constants matching kernel
const IPC_MSG_DATA_SIZE: usize = 64;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Message {
    tag: u32,
    data: [u8; IPC_MSG_DATA_SIZE],
}

impl Message {
    fn new(tag: u32, data: &[u8]) -> Self {
        let mut msg = Self { tag, data: [0; IPC_MSG_DATA_SIZE] };
        let copy_len = data.len().min(IPC_MSG_DATA_SIZE);
        msg.data[..copy_len].copy_from_slice(&data[..copy_len]);
        msg
    }
}

// Well-known endpoint IDs
const FB_SERVER: usize = 1;

// Framebuffer message tags
const FB_WRITE_CHAR: u32 = 1;
const FB_BACKSPACE: u32 = 2;
const FB_CLEAR_SCREEN: u32 = 3;
const FB_WRITE_STRING: u32 = 4;

// IPC syscall numbers
const SYS_IPC_SEND: u64 = 100;

unsafe fn ipc_send(endpoint: usize, msg: &Message) -> Result<(), ()> {
    let result = syscall(
        SYS_IPC_SEND,
        endpoint as u64,
        msg as *const Message as u64,
        0,
        0,
        0,
    );
    match result {
        0 => Ok(()),
        _ => Err(()),
    }
}

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

fn fb_send_char(byte: u8) {
    let msg = Message::new(FB_WRITE_CHAR, &[byte]);
    unsafe { let _ = ipc_send(FB_SERVER, &msg); }
}

fn fb_backspace() {
    let msg = Message::new(FB_BACKSPACE, &[]);
    unsafe { let _ = ipc_send(FB_SERVER, &msg); }
}

fn fb_clear_screen() {
    let msg = Message::new(FB_CLEAR_SCREEN, &[]);
    unsafe { let _ = ipc_send(FB_SERVER, &msg); }
}

fn fb_write_str(s: &str) {
    let msg = Message::new(FB_WRITE_STRING, s.as_bytes());
    unsafe { let _ = ipc_send(FB_SERVER, &msg); }
}

struct Stdout;

impl core::fmt::Write for Stdout {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for byte in s.bytes() {
            fb_send_char(byte);
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

unsafe fn sys_yield() -> u64 {
    syscall(7, 0, 0, 0, 0, 0)
}

unsafe fn sys_read_ticks() -> u64 {
    syscall(3, 0, 0, 0, 0, 0)
}

unsafe fn sys_read_scancode() -> Option<u8> {
    let scancode = syscall(1, 0, 0, 0, 0, 0);
    if scancode == u64::MAX {
        None
    } else {
        Some(scancode as u8)
    }
}

unsafe fn sys_exit() -> ! {
    syscall(8, 0, 0, 0, 0, 0);
    loop {}
}

unsafe fn sys_poweroff() {
    syscall(4, 0, 0, 0, 0, 0);
}

unsafe fn sys_date() -> (u8, u8, u8, u8, u8, u8) {
    let packed = syscall(5, 0, 0, 0, 0, 0);
    (
        (packed & 0xFF) as u8,
        ((packed >> 8) & 0xFF) as u8,
        ((packed >> 16) & 0xFF) as u8,
        ((packed >> 24) & 0xFF) as u8,
        ((packed >> 32) & 0xFF) as u8,
        ((packed >> 40) & 0xFF) as u8,
    )
}

unsafe fn sys_lspci() {
    syscall(6, 0, 0, 0, 0, 0);
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    print!("> ");

    let mut keyboard = pc_keyboard::Keyboard::new(
        pc_keyboard::ScancodeSet1::new(),
        pc_keyboard::layouts::Us104Key,
        pc_keyboard::HandleControl::Ignore,
    );

    let mut buffer = [0u8; 128];
    let mut len = 0;

    let mut cursor_visible = false;
    let mut last_blink = unsafe { sys_read_ticks() };

    loop {
        if let Some(scancode) = unsafe { sys_read_scancode() } {
            if cursor_visible {
                fb_backspace();
                cursor_visible = false;
            }

            if let Ok(Some(key_event)) = keyboard.add_byte(scancode) {
                if let Some(key) = keyboard.process_keyevent(key_event) {
                    match key {
                        pc_keyboard::DecodedKey::Unicode(character) => match character {
                            '\n' => {
                                println!();
                                if let Ok(s) = core::str::from_utf8(&buffer[..len]) {
                                    process_command(s);
                                }
                                unsafe { sys_yield() };
                                len = 0;
                                print!("> ");
                            }
                            '\u{8}' | '\x7f' => {
                                if len > 0 {
                                    len -= 1;
                                    fb_backspace();
                                }
                            }
                            c if c.is_ascii_graphic() || c == ' ' => {
                                if len < buffer.len() {
                                    let mut buf = [0; 4];
                                    let s = c.encode_utf8(&mut buf);
                                    if len + s.len() <= buffer.len() {
                                        for b in s.bytes() {
                                            buffer[len] = b;
                                            len += 1;
                                        }
                                        print!("{}", c);
                                    }
                                }
                            }
                            _ => {}
                        },
                        pc_keyboard::DecodedKey::RawKey(key) => match key {
                            pc_keyboard::KeyCode::Backspace => {
                                if len > 0 {
                                    len -= 1;
                                    fb_backspace();
                                }
                            }
                            _ => {}
                        },
                    }
                }
            }
            last_blink = unsafe { sys_read_ticks() };
        } else {
            let now = unsafe { sys_read_ticks() };
            if now.wrapping_sub(last_blink) >= 9 {
                if cursor_visible {
                    fb_backspace();
                } else {
                    fb_send_char(b'_');
                }
                cursor_visible = !cursor_visible;
                last_blink = now;
            }
            core::hint::spin_loop();
        }
    }
}

fn process_command(cmd: &str) {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return;
    }

    let mut parts = cmd.split_whitespace();
    let command = parts.next().unwrap_or("");

    match command {
        "help" => {
            println!("Available commands:");
            println!("  help     - Show this help message");
            println!("  echo     - Print the arguments");
            println!("  clear    - Clear the screen");
            println!("  poweroff - Shutdown the system");
            println!("  date     - Print the current date and time");
            println!("  lspci    - List all PCI devices");
            println!("  exit     - Exit the shell");
        }
        "echo" => {
            let rest = cmd["echo".len()..].trim();
            println!("{}", rest);
        }
        "clear" => {
            fb_clear_screen();
        }
        "poweroff" => {
            unsafe { sys_poweroff() };
        }
        "date" => {
            let (year, month, day, hours, minutes, seconds) = unsafe { sys_date() };
            println!(
                "{:02}:{:02}:{:02} {:04}-{:02}-{:02}",
                hours, minutes, seconds,
                2000 + (year as u16), month, day
            );
        }
        "lspci" => {
            unsafe { sys_lspci() };
        }
        "exit" => {
            unsafe { sys_exit() };
        }
        _ => {
            println!("Unknown command: {}", command);
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
