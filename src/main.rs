#![no_std]
#![no_main]

use core::panic::PanicInfo;

// This function is called on panic (yes, I'm even copying the comments)
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

static MESSAGE: &[u8] = b"Fieletowy!";

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // start
    // What start does everyone sees
    // (start, co robi, każdy widzi)
    // (koń, jaki jest, każdy widzi)

    let vga_buffer = 0xb8000 as *mut u8;

    for (i, &byte) in MESSAGE.iter().enumerate() {
        unsafe {
            *vga_buffer.offset(i as isize * 2) = byte;
            *vga_buffer.offset(i as isize * 2 + 1) = 0xd;
        }
    }

    loop {}
}
