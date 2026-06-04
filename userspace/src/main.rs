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
    println!("Hello from Ring 3!");
    println!("The userspace shell is alive!");
    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
