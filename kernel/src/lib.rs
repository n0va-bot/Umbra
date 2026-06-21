#![no_std]
#![cfg_attr(test, no_main)]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]
#![feature(abi_x86_interrupt)]

use core::panic::PanicInfo;

extern crate alloc;

// Import macros
#[macro_use]
extern crate kernel_arch;

// Re-export from kernel-arch
pub use kernel_arch::acpi;
pub use kernel_arch::cmos;
pub use kernel_arch::gdt;
pub use kernel_arch::interrupts;
pub use kernel_arch::memory;
pub use kernel_arch::pci;
pub use kernel_arch::serial;
pub use kernel_arch::syscall;
pub use kernel_arch::userspace;

// Re-export from kernel-core
pub use kernel_core::allocator;
pub use kernel_core::elf_loader;
pub use kernel_core::ipc;
pub use kernel_core::process;
pub use kernel_core::syscall as core_syscall;
pub use kernel_core::tar;
pub use kernel_core::task;

pub mod arch {
    pub fn init() {
        kernel_arch::gdt::init();
        kernel_arch::interrupts::init_idt();
        unsafe { kernel_arch::interrupts::PICS.lock().initialize() };
        x86_64::instructions::interrupts::enable();
    }
}

pub fn init() {
    arch::init();
}

pub fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
        if crate::interrupts::RESCHEDULE_NEEDED
            .compare_exchange(
                true,
                false,
                core::sync::atomic::Ordering::AcqRel,
                core::sync::atomic::Ordering::Acquire,
            )
            .is_ok()
        {
            if let Some(next) = crate::process::schedule(0) {
                unsafe { crate::process::switch_to(0, next) };
            }
        }
    }
}

pub trait Testable {
    fn run(&self) -> ();
}

impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        serial_print!("{}...\t", core::any::type_name::<T>());
        self();
        serial_println!("[ok]");
    }
}

pub fn test_runner(tests: &[&dyn Testable]) {
    serial_println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }
    exit_qemu(QemuExitCode::Success);
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    test_panic_handler(info)
}

#[cfg(test)]
use bootloader_api::config::{BootloaderConfig, Mapping};
#[cfg(test)]
use bootloader_api::{BootInfo, entry_point};

#[cfg(test)]
pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

#[cfg(test)]
entry_point!(test_kernel_main, config = &BOOTLOADER_CONFIG);

#[cfg(test)]
fn test_kernel_main(_boot_info: &'static mut BootInfo) -> ! {
    init();
    test_main();
    hlt_loop();
}

#[test_case]
fn test_breakpoint_exception() {
    x86_64::instructions::interrupts::int3();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

pub fn test_panic_handler(info: &PanicInfo) -> ! {
    serial_println!("[failed]\n");
    serial_println!("Error: {}\n", info);
    exit_qemu(QemuExitCode::Failed);
    hlt_loop();
}

pub fn exit_qemu(exit_code: QemuExitCode) {
    use x86_64::instructions::port::Port;

    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
}
