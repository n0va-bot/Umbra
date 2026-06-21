#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(umbra::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use bootloader_api::config::{BootloaderConfig, Mapping};
use bootloader_api::{BootInfo, entry_point};
use core::panic::PanicInfo;
use umbra::process::{self, PROCESSES, State};
use umbra::task::executor::Executor;
use x86_64::VirtAddr;

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

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

    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset.into_option().unwrap());

    umbra::arch::init();
    umbra::serial_println!("[kernel] init done");

    let fb_vaddr = VirtAddr::new(framebuffer.buffer().as_ptr() as u64);
    let fb_paddr = unsafe { umbra::memory::translate_addr(fb_vaddr, phys_mem_offset).unwrap() };
    *umbra::syscall::FB_INFO.lock() = Some(umbra::syscall::SysFbInfo {
        phys_addr: fb_paddr.as_u64(),
        byte_len: framebuffer.buffer().len(),
        width: info.width,
        height: info.height,
        pixel_format: match info.pixel_format {
            bootloader_api::info::PixelFormat::Rgb => 0,
            bootloader_api::info::PixelFormat::Bgr => 1,
            bootloader_api::info::PixelFormat::U8 => 2,
            _ => 3, // Unknown
        },
        bytes_per_pixel: info.bytes_per_pixel,
        stride: info.stride,
    });
    umbra::serial_println!(
        "[kernel] framebuffer info saved (paddr: {:#X})",
        fb_paddr.as_u64()
    );

    let ramdisk_addr = boot_info
        .ramdisk_addr
        .into_option()
        .expect("No ramdisk found");
    let ramdisk_len = boot_info.ramdisk_len as usize;

    let mut _mapper = unsafe { umbra::memory::init_all(boot_info) };

    umbra::acpi::init(phys_mem_offset.as_u64());
    umbra::serial_println!("[kernel] acpi initialized");
    umbra::syscall::init();
    umbra::ipc::init();
    umbra::serial_println!("[kernel] ipc initialized");

    let kernel_index = umbra::process::init(ramdisk_addr, ramdisk_len);

    let mut executor = Executor::new();

    loop {
        {
            let mut allocator_guard = umbra::memory::FRAME_ALLOCATOR.lock();
            let allocator = allocator_guard.as_mut().unwrap();
            process::teardown_exited(allocator);
        }

        match process::schedule(kernel_index) {
            Some(next_idx) => {
                {
                    let mut table = PROCESSES.lock();
                    if let Some(p) = table.get_mut(next_idx) {
                        p.state = State::Running;
                    }
                }

                unsafe { process::switch_to(kernel_index, next_idx) };

                {
                    let mut table = PROCESSES.lock();
                    if let Some(p) = table.get_mut(next_idx) {
                        if p.state == State::Running {
                            p.state = State::Ready;
                        }
                    }
                }
            }
            None => {
                executor.run_ready_tasks();
                x86_64::instructions::interrupts::enable_and_hlt();
            }
        }
    }

    #[cfg(test)]
    test_main();

    let mut executor = Executor::new();
}

// This function is called on panic (yes, I'm even copying the comments)
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    umbra::serial_println!("[PANIC] {}", info);
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
