#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(umbra::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use bootloader_api::config::{BootloaderConfig, Mapping};
use bootloader_api::{BootInfo, entry_point};

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};
use core::panic::PanicInfo;
use umbra::elf_loader::load_elf;
use umbra::memory;
use umbra::memory::BootInfoFrameAllocator;
use umbra::task::keyboard;
use umbra::task::{Task, executor::Executor};
use umbra::{print, println};
use x86_64::VirtAddr;
use x86_64::structures::paging::FrameAllocator;

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    // start
    // What start does everyone sees
    // (start, co robi, każdy widzi)
    // (koń, jaki jest, każdy widzi)

    umbra::serial_println!("[kernel] entered kernel_main");

    let framebuffer = boot_info.framebuffer.as_mut().unwrap();
    let info = framebuffer.info();
    umbra::serial_println!(
        "[kernel] framebuffer: {}x{}, {:?}, bpp={}",
        info.width,
        info.height,
        info.pixel_format,
        info.bytes_per_pixel
    );
    umbra::framebuffer::init(framebuffer.buffer_mut(), info);
    umbra::serial_println!("[kernel] framebuffer initialized");

    println!("Hello World{}", "!");
    umbra::serial_println!("[kernel] println done");

    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset.into_option().unwrap());

    umbra::init();
    umbra::serial_println!("[kernel] init done");

    let mut mapper = unsafe { memory::init(phys_mem_offset) };
    let mut frame_allocator =
        unsafe { BootInfoFrameAllocator::init(&mut boot_info.memory_regions) };

    umbra::allocator::init_heap(&mut mapper, &mut frame_allocator)
        .expect("heap initialization failed");
    umbra::serial_println!("[kernel] heap initialized");

    umbra::acpi::init(phys_mem_offset.as_u64());
    umbra::serial_println!("[kernel] acpi initialized");
    umbra::syscall::init();

    // Load userspace shell
    #[repr(C, align(8))]
    struct Aligned<T: ?Sized>(T);

    static SHELL_ELF: &Aligned<[u8]> = &Aligned(*include_bytes!(
        "../../userspace/target/x86_64-unknown-none/debug/userspace"
    ));

    let entry_point = umbra::elf_loader::load_elf(&SHELL_ELF.0, &mut mapper, &mut frame_allocator);

    // Allocate user stack
    unsafe {
        use x86_64::structures::paging::{Mapper, Page, PageTableFlags};

        let stack_page = Page::containing_address(VirtAddr::new(0x5555_0000_0000));
        let stack_frame = frame_allocator.allocate_frame().unwrap();
        let flags =
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;

        mapper
            .map_to(stack_page, stack_frame, flags, &mut frame_allocator)
            .expect("map_to failed")
            .flush();

        let stack_ptr = stack_page.start_address().as_u64() + 4096;
        let code_sel = umbra::gdt::get_user_code_selector().0 as u64;
        let data_sel = umbra::gdt::get_user_data_selector().0 as u64;

        umbra::userspace::enter_user_mode(entry_point, stack_ptr, code_sel, data_sel);
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
    umbra::serial_println!("[PANIC] {}", info);
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
