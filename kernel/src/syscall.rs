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
    rsi: u64,
    rdx: u64,
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
        let pending = crate::interrupts::PENDING_NEXT.swap(usize::MAX, Ordering::AcqRel);
        let current = crate::process::CURRENT_PROCESS.load(Ordering::SeqCst);
        if current != 0 && pending != usize::MAX && pending != current {
            {
                let mut table = crate::process::PROCESSES.lock();
                if let Some(p) = table.get_mut(current) {
                    p.state = crate::process::State::Ready;
                }
            }
            unsafe { crate::process::switch_to(current, pending) };
        }
    }

    match syscall_nr {
        1 => {
            if let Ok(queue) = crate::task::keyboard::SCANCODE_QUEUE.try_get() {
                if let Some(scancode) = queue.pop() {
                    return scancode as u64;
                }
            }
            u64::MAX
        }
        3 => crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed),
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

        // IPC syscalls
        100 => {
            // ipc_send(endpoint_id, message_ptr)
            // rdi = endpoint_id, rsi = pointer to Message struct in user space
            let endpoint_id = rdi as usize;
            let msg_ptr = rsi as *const crate::ipc::Message;
            
            // Validate the message pointer is in user space
            let msg_vaddr = x86_64::VirtAddr::new(rsi);
            if crate::process::validate_user_range(msg_vaddr, core::mem::size_of::<crate::ipc::Message>()).is_none() {
                crate::serial_println!("[syscall] 100: rejected invalid message pointer {:#X}", rsi);
                return u64::MAX;
            }
            
            let msg = unsafe { &*msg_ptr };
            let endpoint_id = crate::ipc::EndpointId(endpoint_id);
            
            let mut ipc_state = crate::ipc::IPC.lock();
            
            // Check if this is a message for an in-kernel service
            if ipc_state.handle_kernel_service(endpoint_id, msg) {
                return 0;
            }
            
            // Otherwise, send to the endpoint registry
            match ipc_state.send(endpoint_id, *msg) {
                Ok(_) => 0,
                Err(crate::ipc::SendError::InvalidEndpoint) => u64::MAX,
                Err(crate::ipc::SendError::QueueFull(_)) => u64::MAX - 1,
            }
        }
        101 => {
            // ipc_recv(endpoint_id, message_ptr)
            // rdi = endpoint_id, rsi = pointer to Message struct in user space (output)
            let endpoint_id = crate::ipc::EndpointId(rdi as usize);
            let msg_ptr = rsi as *mut crate::ipc::Message;
            
            // Validate the message pointer is in user space
            let msg_vaddr = x86_64::VirtAddr::new(rsi);
            if crate::process::validate_user_range(msg_vaddr, core::mem::size_of::<crate::ipc::Message>()).is_none() {
                crate::serial_println!("[syscall] 101: rejected invalid message pointer {:#X}", rsi);
                return u64::MAX;
            }
            
            let mut ipc_state = crate::ipc::IPC.lock();
            match ipc_state.recv(endpoint_id) {
                Some(msg) => {
                    unsafe { *msg_ptr = msg };
                    0
                }
                None => {
                    // Block the process until a message arrives
                    let current = crate::process::CURRENT_PROCESS.load(Ordering::SeqCst);
                    if current != 0 {
                        // Mark as blocked and switch to kernel
                        {
                            let mut table = crate::process::PROCESSES.lock();
                            if let Some(p) = table.get_mut(current) {
                                p.state = crate::process::State::Blocked;
                            }
                        }
                        unsafe { crate::process::switch_to(current, 0) };
                    }
                    u64::MAX // Indicate no message (shouldn't reach here after block)
                }
            }
        }
        102 => {
            // ipc_call(endpoint_id, send_msg_ptr, recv_msg_ptr)
            // rdi = endpoint_id, rsi = send message ptr, rdx = recv message ptr
            // This is send + recv + reply in one shot
            let endpoint_id = crate::ipc::EndpointId(rdi as usize);
            let send_msg_ptr = rsi as *const crate::ipc::Message;
            let recv_msg_ptr = rdx as *mut crate::ipc::Message;
            
            // Validate both pointers
            let send_vaddr = x86_64::VirtAddr::new(rsi);
            let recv_vaddr = x86_64::VirtAddr::new(rdx);
            let msg_size = core::mem::size_of::<crate::ipc::Message>();
            
            if crate::process::validate_user_range(send_vaddr, msg_size).is_none() {
                crate::serial_println!("[syscall] 102: rejected invalid send message pointer {:#X}", rsi);
                return u64::MAX;
            }
            if crate::process::validate_user_range(recv_vaddr, msg_size).is_none() {
                crate::serial_println!("[syscall] 102: rejected invalid recv message pointer {:#X}", rdx);
                return u64::MAX;
            }
            
            let send_msg = unsafe { &*send_msg_ptr };
            
            // First send
            {
                let mut ipc_state = crate::ipc::IPC.lock();
                match ipc_state.send(endpoint_id, *send_msg) {
                    Ok(_) => {}
                    Err(_) => return u64::MAX,
                }
            }
            
            // Then receive (blocking)
            let mut ipc_state = crate::ipc::IPC.lock();
            match ipc_state.recv(endpoint_id) {
                Some(msg) => {
                    unsafe { *recv_msg_ptr = msg };
                    0
                }
                None => {
                    // Block the process
                    let current = crate::process::CURRENT_PROCESS.load(Ordering::SeqCst);
                    if current != 0 {
                        {
                            let mut table = crate::process::PROCESSES.lock();
                            if let Some(p) = table.get_mut(current) {
                                p.state = crate::process::State::Blocked;
                            }
                        }
                        unsafe { crate::process::switch_to(current, 0) };
                    }
                    u64::MAX
                }
            }
        }

        _ => 0,
    }
}
