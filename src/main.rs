#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(umbra::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use bootloader::{BootInfo, entry_point};
use core::panic::PanicInfo;
use umbra::memory;
use umbra::memory::BootInfoFrameAllocator;
use umbra::task::keyboard;
use umbra::task::{Task, executor::Executor};
use umbra::{print, println};
use x86_64::VirtAddr;

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    // start
    // What start does everyone sees
    // (start, co robi, każdy widzi)
    // (koń, jaki jest, każdy widzi)

    umbra::init();

    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe { BootInfoFrameAllocator::init(&boot_info.memory_map) };

    umbra::allocator::init_heap(&mut mapper, &mut frame_allocator)
        .expect("heap initialization failed");

    #[cfg(test)]
    test_main();

    let mut executor = Executor::new();
    umbra::vga_buffer::clear_screen();
    umbra::vga_buffer::enable_cursor();
    print!("> ");
    executor.spawn(Task::new(keyboard::run_shell()));
    executor.run();
}

// This function is called on panic (yes, I'm even copying the comments)
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("{}", info);
    umbra::hlt_loop();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    umbra::test_panic_handler(info)
}

#[test_case]
fn useless_test() {
    assert_eq!(1, 1);
}
