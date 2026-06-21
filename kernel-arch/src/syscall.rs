use core::arch::naked_asm;
use core::sync::atomic::{AtomicUsize, Ordering};
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

pub static mut USER_RSP_COPY: u64 = 0;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SysFbInfo {
    pub phys_addr: u64,
    pub byte_len: usize,
    pub width: usize,
    pub height: usize,
    pub pixel_format: u8,
    pub bytes_per_pixel: usize,
    pub stride: usize,
}

pub static FB_INFO: spin::Mutex<Option<SysFbInfo>> = spin::Mutex::new(None);

pub static SYSCALL_HANDLER: AtomicUsize = AtomicUsize::new(0);

// Kernel stack pointer for syscall entry, updated by switch_to
// Must be public so kernel-core can update it
#[unsafe(no_mangle)]
pub static mut KERNEL_RSP: u64 = 0;

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
        "mov r9, rax",
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
        kernel_rsp = sym KERNEL_RSP,
        dispatch = sym syscall_dispatch,
    );
}

extern "C" fn syscall_dispatch(
    rdi: u64,
    rsi: u64,
    rdx: u64,
    rcx: u64,
    r8: u64,
    syscall_nr: u64,
) -> u64 {
    let handler_ptr = SYSCALL_HANDLER.load(Ordering::SeqCst);
    if handler_ptr != 0 {
        let handler: extern "C" fn(u64, u64, u64, u64, u64, u64) -> u64 =
            unsafe { core::mem::transmute(handler_ptr) };
        handler(rdi, rsi, rdx, rcx, r8, syscall_nr)
    } else {
        u64::MAX
    }
}
