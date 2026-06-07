#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

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

pub fn sys_write(byte: u8) {
    unsafe { syscall(0, byte as u64, 0, 0, 0, 0) };
}

unsafe fn sys_yield() -> u64 {
    syscall(7, 0, 0, 0, 0, 0)
}

struct Stdout;

impl core::fmt::Write for Stdout {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for b in s.bytes() {
            sys_write(b);
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
    let mut last_blink = unsafe { syscall(3, 0, 0, 0, 0, 0) };

    loop {
        let scancode = unsafe { syscall(1, 0, 0, 0, 0, 0) };
        if scancode != core::u64::MAX {
            if cursor_visible {
                print!("\u{8}");
                cursor_visible = false;
            }

            if let Ok(Some(key_event)) = keyboard.add_byte(scancode as u8) {
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
                                    print!("\u{8}");
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
                                    print!("\u{8}");
                                }
                            }
                            _ => {}
                        },
                    }
                }
            }
            last_blink = unsafe { syscall(3, 0, 0, 0, 0, 0) };
        } else {
            let now = unsafe { syscall(3, 0, 0, 0, 0, 0) };
            if now.wrapping_sub(last_blink) >= 9 {
                if cursor_visible {
                    print!("\u{8}");
                } else {
                    print!("_");
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
            unsafe { syscall(2, 0, 0, 0, 0, 0) };
        }
        "poweroff" => {
            unsafe { syscall(4, 0, 0, 0, 0, 0) };
        }
        "date" => {
            let packed = unsafe { syscall(5, 0, 0, 0, 0, 0) };
            let year = packed & 0xFF;
            let month = (packed >> 8) & 0xFF;
            let day = (packed >> 16) & 0xFF;
            let hours = (packed >> 24) & 0xFF;
            let minutes = (packed >> 32) & 0xFF;
            let seconds = (packed >> 40) & 0xFF;
            println!(
                "{:02}:{:02}:{:02} {:04}-{:02}-{:02}",
                hours,
                minutes,
                seconds,
                2000 + (year as u16),
                month,
                day
            );
        }
        "lspci" => {
            unsafe { syscall(6, 0, 0, 0, 0, 0) };
        }
        "exit" => {
            unsafe { syscall(8, 0, 0, 0, 0, 0) };
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
