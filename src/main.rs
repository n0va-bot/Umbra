#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // start
    // What start does everyone sees
    // (start, co robi, każdy widzi)
    loop {}
}

// This function is called on panic (yes, I'm even copying the comments)
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
