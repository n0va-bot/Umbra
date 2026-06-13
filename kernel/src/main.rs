#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(umbra::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use bootloader_api::config::{BootloaderConfig, Mapping};
use bootloader_api::{BootInfo, entry_point};
use core::panic::PanicInfo;
use umbra::memory::BootInfoFrameAllocator;
use umbra::println;
use umbra::process::{self, PROCESSES, Pid, Process, SavedRegs, State};
use umbra::task::executor::Executor;
use x86_64::VirtAddr;
use x86_64::registers::control::Cr3;

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
    umbra::framebuffer::init(framebuffer.buffer_mut(), info);
    umbra::serial_println!("[kernel] framebuffer initialized");

    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset.into_option().unwrap());

    umbra::init();
    umbra::serial_println!("[kernel] init done");

    let mut mapper = unsafe { umbra::memory::init(phys_mem_offset) };
    let mut frame_allocator =
        unsafe { BootInfoFrameAllocator::init(&mut boot_info.memory_regions) };

    umbra::allocator::init_heap(&mut mapper, &mut frame_allocator)
        .expect("heap initialization failed");
    umbra::serial_println!("[kernel] heap initialized");

    umbra::memory::store_phys_mem_offset(phys_mem_offset);
    umbra::serial_println!(
        "[kernel] phys_mem_offset stored at {:#X}",
        phys_mem_offset.as_u64()
    );

    umbra::acpi::init(phys_mem_offset.as_u64());
    umbra::serial_println!("[kernel] acpi initialized");
    umbra::syscall::init();
    umbra::ipc::init();
    umbra::serial_println!("[kernel] ipc initialized");
    umbra::task::keyboard::ScancodeStream::init_scancode_queue();

    // Load userspace shell
    let (boot_frame, _) = Cr3::read();
    let (kernel_stack_slot, kernel_stack_top) = process::allocate_kernel_stack();

    let kernel_process = Process {
        pid: Pid::alloc(),
        state: State::Running,
        cr3: boot_frame.start_address(),
        kernel_stack_top,
        kernel_stack_slot,
        kernel_rsp: VirtAddr::new(0),
        saved: SavedRegs::default(),
        interrupt_frame: process::InterruptFrame::default(),
    };

    let kernel_index = {
        let mut table = PROCESSES.lock();
        let index = table.insert(kernel_process);
        table.set_current(index);
        index
    };

    let ramdisk_addr = boot_info
        .ramdisk_addr
        .into_option()
        .expect("No ramdisk found");
    let ramdisk_len = boot_info.ramdisk_len as usize;
    let ramdisk = unsafe { core::slice::from_raw_parts(ramdisk_addr as *const u8, ramdisk_len) };

    let archive = umbra::tar::TarArchive::new(ramdisk);
    for entry in archive.iter() {
        umbra::serial_println!("[kernel] found in initramfs: {}, size: {}", entry.name, entry.size);
        if entry.size > 0 {
            process::spawn(entry.data, &mut frame_allocator);
            umbra::serial_println!("[kernel] {} spawned", entry.name);
        }
    }

    let mut executor = Executor::new();

    loop {
        process::teardown_exited(&mut frame_allocator);

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
