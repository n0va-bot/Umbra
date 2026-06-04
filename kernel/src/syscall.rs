use core::arch::naked_asm;
use x86_64::registers::model_specific::{Efer, EferFlags, LStar, SFMask, Star};
use x86_64::structures::gdt::SegmentSelector;

pub fn init() {
    unsafe {
        Efer::update(|flags| flags.insert(EferFlags::SYSTEM_CALL_EXTENSIONS));

        let user_cs = crate::gdt::get_user_code_selector();
        let user_ds = crate::gdt::get_user_data_selector();
        let kernel_cs = crate::gdt::get_kernel_code_selector();
        let kernel_ds = crate::gdt::get_kernel_data_selector();

        Star::write(user_cs, user_ds, kernel_cs, kernel_ds).unwrap();

        LStar::write(x86_64::VirtAddr::new(syscall_entry as u64));

        SFMask::write(x86_64::registers::rflags::RFlags::INTERRUPT_FLAG);
    }
}

static mut KERNEL_SYSCALL_STACK: [u8; 4096 * 4] = [0; 4096 * 4];
static mut USER_RSP_BACKUP: u64 = 0;

#[unsafe(naked)]
extern "C" fn syscall_entry() {
    naked_asm!(
        // Swap to kernel stack
        "mov [rip + {user_rsp}], rsp",
        "lea rsp, [rip + {kernel_stack} + {stack_size}]",

        // Save registers (SysV)
        "push r11",
        "push rcx",
        "push rdi",
        "push rsi",
        "push rdx",
        "push r10",
        "push r8",
        "push r9",

        "mov rcx, r10",

        "call {dispatch}",
        "pop r9",
        "pop r8",
        "pop r10",
        "pop rdx",
        "pop rsi",
        "pop rdi",
        "pop rcx",
        "pop r11",

        // Swap back to user stack
        "mov rsp, [rip + {user_rsp}]",

        // Return to userspace
        "sysretq",

        user_rsp = sym USER_RSP_BACKUP,
        kernel_stack = sym KERNEL_SYSCALL_STACK,
        stack_size = const 4096 * 4,
        dispatch = sym syscall_dispatch,
    );
}

extern "C" fn syscall_dispatch(rdi: u64, rsi: u64, rdx: u64, rcx: u64, r8: u64, r9: u64) -> u64 {
    let syscall_nr: u64;
    unsafe { core::arch::asm!("mov {}, rax", out(reg) syscall_nr) };

    match syscall_nr {
        0 => {
            if rdi as u8 == 8 {
                crate::framebuffer::backspace();
            } else {
                crate::print!("{}", rdi as u8 as char);
            }
            0
        }
        1 => {
            if let Ok(queue) = crate::task::keyboard::SCANCODE_QUEUE.try_get() {
                if let Some(scancode) = queue.pop() {
                    return scancode as u64;
                }
            }
            u64::MAX
        }
        2 => {
            crate::framebuffer::clear_screen();
            0
        }
        3 => crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed),
        4 => {
            crate::acpi::power_off();
            0
        }
        5 => {
            let mut cmos = crate::cmos::Cmos::new();
            let (year, month, day, hours, minutes, seconds) = cmos.read_time();
            (year as u64)
                | ((month as u64) << 8)
                | ((day as u64) << 16)
                | ((hours as u64) << 24)
                | ((minutes as u64) << 32)
                | ((seconds as u64) << 40)
        }
        6 => {
            crate::pci::scan_buses();
            0
        }
        _ => 0,
    }
}
