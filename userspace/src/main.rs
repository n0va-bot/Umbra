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

const FB_SERVER: usize = 11;
const RTC_SERVER: usize = 12;
const PCI_SERVER: usize = 13;
const POWER_SERVER: usize = 14;

const FB_WRITE_CHAR: u32 = 1;
const FB_BACKSPACE: u32 = 2;
const FB_CLEAR_SCREEN: u32 = 3;
const FB_WRITE_STRING: u32 = 4;
const RTC_GET_TIME: u32 = 1;
const PCI_SCAN_BUSES: u32 = 1;
const POWER_OFF: u32 = 1;

const SYS_IPC_SEND: u64 = 100;
const SYS_IPC_RECV: u64 = 101;
const SYS_IPC_CALL: u64 = 102;
const SYS_IPC_CREATE_ENDPOINT: u64 = 104;

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
        if result == 0 { return Ok(()); }
        if result == u64::MAX { return Err(()); }
        // Queue full, yield and retry
        unsafe { sys_yield() };
    }
}

fn lookup_service(name: &str, reply_endpoint: usize) -> Option<usize> {
    let mut req = Message::empty();
    req.tag = 2; // LOOKUP_SERVICE
    let bytes = name.as_bytes();
    let copy_len = bytes.len().min(8);
    req.data[0..copy_len].copy_from_slice(&bytes[..copy_len]);
    req.data[8..16].copy_from_slice(&(reply_endpoint as u64).to_le_bytes());

    let _ = ipc_send(1, &req); // SerV is at 1

    let mut resp = Message::empty();
    if ipc_recv(reply_endpoint, &mut resp).is_ok() {
        if resp.tag == 2 {
            let ep = usize::from_le_bytes(resp.data[0..8].try_into().unwrap());
            if ep != usize::MAX {
                return Some(ep);
            }
        }
    }
    None
}

fn fb_send_char(byte: u8) {
    let msg = Message::new(FB_WRITE_CHAR, &[byte]);
    let _ = ipc_send(FB_SERVER, &msg);
}
fn fb_backspace() {
    let msg = Message::new(FB_BACKSPACE, &[]);
    let _ = ipc_send(FB_SERVER, &msg);
}
fn fb_clear_screen() {
    let msg = Message::new(FB_CLEAR_SCREEN, &[]);
    let _ = ipc_send(FB_SERVER, &msg);
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

fn sys_read_ticks(tick_server: usize, my_ep: usize) -> u64 {
    let mut req = Message::new(1, &[]); // TICK_GET
    req.data[0..8].copy_from_slice(&(my_ep as u64).to_le_bytes());
    let _ = ipc_send(tick_server, &req);

    let mut resp = Message::empty();
    if ipc_recv(my_ep, &mut resp).is_ok() {
        if resp.tag == 1 {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&resp.data[..8]);
            return u64::from_le_bytes(bytes);
        }
    }
    0
}

fn sys_read_char(kb_server: usize, my_ep: usize) -> Option<char> {
    // Use KB_GET_CHAR_POLL to avoid blocking
    let mut req = Message::new(3, &[]); // KB_GET_CHAR_POLL
    req.data[0..8].copy_from_slice(&(my_ep as u64).to_le_bytes());
    let _ = ipc_send(kb_server, &req);

    let mut resp = Message::empty();
    if ipc_recv(my_ep, &mut resp).is_ok() {
        if resp.tag == 3 {
            let ch_u32 = u32::from_le_bytes(resp.data[0..4].try_into().unwrap());
            if ch_u32 == u32::MAX {
                return None;
            }
            if ch_u32 == 8 {
                return Some('\x08'); // Backspace
            }
            if let Some(c) = core::char::from_u32(ch_u32) {
                return Some(c);
            }
        }
    }
    None
}

unsafe fn sys_exit() -> ! {
    syscall(8, 0, 0, 0, 0, 0);
    loop {}
}

fn sys_poweroff() {
    let msg = Message::new(POWER_OFF, &[]);
    let _ = ipc_send(POWER_SERVER, &msg);
}

fn sys_date(my_ep: usize) {
    let mut req = Message::new(RTC_GET_TIME, &[]);
    req.data[0..8].copy_from_slice(&(my_ep as u64).to_le_bytes());
    let _ = ipc_send(RTC_SERVER, &req);

    let mut resp = Message::empty();
    if ipc_recv(my_ep, &mut resp).is_ok() && resp.tag == 1 {
        let year = resp.data[0];
        let month = resp.data[1];
        let day = resp.data[2];
        let hours = resp.data[3];
        let minutes = resp.data[4];
        let seconds = resp.data[5];

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
    print!("{}", core::str::from_utf8(&buf[..digits as usize]).unwrap());
}

fn sys_lspci(my_ep: usize) {
    let mut req = Message::new(PCI_SCAN_BUSES, &[]);
    req.data[0..8].copy_from_slice(&(my_ep as u64).to_le_bytes());
    let _ = ipc_send(PCI_SERVER, &req);

    loop {
        let mut resp = Message::empty();
        if ipc_recv(my_ep, &mut resp).is_ok() {
            if resp.tag == 1 {
                let bus = resp.data[0];
                let device = resp.data[1];
                let func = resp.data[2];
                let class_code = resp.data[3];
                let subclass = resp.data[4];
                let vendor_id = u16::from_le_bytes(resp.data[5..7].try_into().unwrap());
                let device_id = u16::from_le_bytes(resp.data[7..9].try_into().unwrap());

                print!("Bus ");
                write_hex(bus as u32, 2);
                print!(" | Dev ");
                write_hex(device as u32, 2);
                print!(" | Func ");
                write_hex(func as u32, 2);
                print!(" => Vendor: ");
                write_hex(vendor_id as u32, 4);
                print!(", Device: ");
                write_hex(device_id as u32, 4);
                print!(" | Class: ");
                write_hex(class_code as u32, 2);
                print!(", Sub: ");
                write_hex(subclass as u32, 2);
                println!();
            } else if resp.tag == 2 {
                break;
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let my_ep = create_endpoint().expect("userspace: failed to create endpoint");

    let kb_server = lookup_service("keyboard", my_ep).expect("userspace: kb-server not found");
    let tick_server =
        lookup_service("tick\0\0\0\0", my_ep).expect("userspace: tick-server not found");

    print!("> ");

    let mut buffer = [0u8; 128];
    let mut len = 0;

    let mut cursor_visible = false;
    let mut last_blink = sys_read_ticks(tick_server, my_ep);

    loop {
        if let Some(c) = sys_read_char(kb_server, my_ep) {
            if cursor_visible {
                fb_backspace();
                cursor_visible = false;
            }

            match c {
                '\n' => {
                    println!();
                    if let Ok(s) = core::str::from_utf8(&buffer[..len]) {
                        process_command(s, my_ep);
                    }
                    unsafe { sys_yield() };
                    len = 0;
                    print!("> ");
                }
                '\x08' | '\x7f' => {
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
            }
            last_blink = sys_read_ticks(tick_server, my_ep);
        } else {
            let now = sys_read_ticks(tick_server, my_ep);
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

fn process_command(cmd: &str, my_ep: usize) {
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
            sys_poweroff();
        }
        "date" => {
            sys_date(my_ep);
        }
        "lspci" => {
            sys_lspci(my_ep);
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
