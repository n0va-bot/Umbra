#![no_std]
#![no_main]

mod vga_buffer;

use core::panic::PanicInfo;

// This function is called on panic (yes, I'm even copying the comments)
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("{}", info);
    loop {}
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // start
    // What start does everyone sees
    // (start, co robi, każdy widzi)
    // (koń, jaki jest, każdy widzi)

    println!("Fieletowy!");

    loop {}
}
