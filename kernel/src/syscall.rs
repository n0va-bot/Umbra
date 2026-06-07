use core::arch::naked_asm;
use core::sync::atomic::Ordering;
use x86_64::registers::model_specific::{Efer, EferFlags, LStar, SFMask, Star};

pub fn init() {
    unsafe {
        Efer::update(|flags| flags.insert(EferFlags::SYSTEM_CALL_EXTENSIONS));

        let kernel_cs = crate::gdt::get_kernel_code_selector();
        let kernel_ds = crate::gdt::get_kernel_data_selector();

        let user_cs = crate::gdt::get_user_code_selector();
        let user_ds = crate::gdt::get_user_data_selector();

        Star::write(user_cs, user_ds, kernel_cs, kernel_ds).unwrap();
        LStar::write(x86_64::VirtAddr::new(syscall_entry as *const () as u64));
        SFMask::write(x86_64::registers::rflags::RFlags::INTERRUPT_FLAG);
    }
}

static mut USER_RSP_COPY: u64 = 0;

#[unsafe(naked)]
extern "C" fn syscall_entry() {
    naked_asm!(
        // Swap to process's kernel stack
        "mov [rip + {user_rsp}], rsp",
        "mov rsp, [rip + {kernel_rsp}]",

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
        "sysretq",

        user_rsp = sym USER_RSP_COPY,
        kernel_rsp = sym crate::process::KERNEL_RSP,
        dispatch = sym syscall_dispatch,
    );
}

extern "C" fn syscall_dispatch(
    rdi: u64,
    _rsi: u64,
    _rdx: u64,
    _rcx: u64,
    _r8: u64,
    _r9: u64,
) -> u64 {
    let syscall_nr: u64;
    unsafe { core::arch::asm!("mov {}, rax", out(reg) syscall_nr) };

    if crate::interrupts::RESCHEDULE_NEEDED
        .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        let current = crate::process::CURRENT_PROCESS.load(Ordering::SeqCst);
        if current != 0 {
            {
                let mut table = crate::process::PROCESSES.lock();
                if let Some(p) = table.get_mut(current) {
                    p.state = crate::process::State::Ready;
                }
            }
            unsafe { crate::process::switch_to(current, 0) };
        }
    }

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
        7 => {
            let current = crate::process::CURRENT_PROCESS.load(Ordering::SeqCst);
            if current != 0 {
                {
                    let mut table = crate::process::PROCESSES.lock();
                    if let Some(p) = table.get_mut(current) {
                        p.state = crate::process::State::Ready;
                    }
                }
                unsafe { crate::process::switch_to(current, 0) };
            }
            0
        }
        8 => {
            let current = crate::process::CURRENT_PROCESS.load(Ordering::SeqCst);
            if current != 0 {
                crate::process::exit(current);
                unsafe { crate::process::switch_to(current, 0) };
            }
            0
        }
        _ => 0,
    }
}
