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
use x86_64::structures::paging::FrameAllocator;

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

    umbra::acpi::init(boot_info.physical_memory_offset);
    umbra::syscall::init();

    unsafe {
        use x86_64::structures::paging::{Mapper, Page, PageTableFlags};

        let code_page = Page::containing_address(VirtAddr::new(0x4444_0000_0000));
        let code_frame = frame_allocator.allocate_frame().unwrap();
        let flags =
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;

        mapper
            .map_to(code_page, code_frame, flags, &mut frame_allocator)
            .expect("map_to failed")
            .flush();

        let shell_code: [u8; 18] = [
            0x48, 0xC7, 0xC7, 0x41, 0x00, 0x00, 0x00, // mov rdi, 0x41 ('A')
            0x48, 0xC7, 0xC0, 0x00, 0x00, 0x00, 0x00, // mov rax, 0 (sys_write)
            0x0F, 0x05, // syscall
            0xEB, 0xFE, // jmp $
        ];
        let code_ptr = code_page.start_address().as_mut_ptr::<u8>();
        for (i, &byte) in shell_code.iter().enumerate() {
            code_ptr.add(i).write(byte);
        }

        let stack_page = Page::containing_address(VirtAddr::new(0x4444_0000_1000));
        let stack_frame = frame_allocator.allocate_frame().unwrap();

        mapper
            .map_to(stack_page, stack_frame, flags, &mut frame_allocator)
            .expect("map_to failed")
            .flush();

        // Jump to Ring 3
        let ip = code_page.start_address().as_u64();
        let stack_ptr = stack_page.start_address().as_u64() + 4096;
        let code_sel = umbra::gdt::get_user_code_selector().0 as u64;
        let data_sel = umbra::gdt::get_user_data_selector().0 as u64;

        umbra::userspace::enter_user_mode(ip, stack_ptr, code_sel, data_sel);
    }

    #[cfg(test)]
    test_main();

    // TODO: restore once we return from userspace via syscall
    // let mut executor = Executor::new();
    // umbra::vga_buffer::clear_screen();
    // umbra::vga_buffer::enable_cursor();
    // print!("> ");
    // executor.spawn(Task::new(keyboard::run_shell()));
    // executor.run();
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
