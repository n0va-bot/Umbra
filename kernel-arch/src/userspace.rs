use core::arch::asm;

#[inline(never)]
pub unsafe fn enter_user_mode(user_rip: u64, user_rsp: u64, user_cs: u64, user_ss: u64) -> ! {
    const RFLAGS_IF: u64 = 1 << 9;

    unsafe {
        asm!(
            "push {ss}",
            "push {rsp}",
            "push {rflags}",
            "push {cs}",
            "push {rip}",
            "iretq",

            ss     = in(reg) user_ss,
            rsp    = in(reg) user_rsp,
            rflags = in(reg) RFLAGS_IF,
            cs     = in(reg) user_cs,
            rip    = in(reg) user_rip,

            options(noreturn)
        );
    }
}
