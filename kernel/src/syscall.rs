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

pub static mut USER_RSP_COPY: u64 = 0;

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
    syscall_nr: u64,
) -> u64 {
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

        10 => {
            let phys_addr = x86_64::PhysAddr::new(rdi);
            let virt_addr = x86_64::VirtAddr::new(rsi);
            let size = rdx as usize;
            match crate::process::map_physical_region(virt_addr, phys_addr, size) {
                Ok(_) => 0,
                Err(_) => u64::MAX,
            }
        }
        11 => {
            let port = rdi as u16;
            unsafe {
                let mut pm = x86_64::instructions::port::Port::<u8>::new(port);
                pm.read() as u64
            }
        }
        12 => {
            let port = rdi as u16;
            let val = rsi as u8;
            unsafe {
                let mut pm = x86_64::instructions::port::Port::<u8>::new(port);
                pm.write(val);
            }
            0
        }
        13 => {
            let port = rdi as u16;
            unsafe {
                let mut pm = x86_64::instructions::port::Port::<u16>::new(port);
                pm.read() as u64
            }
        }
        14 => {
            let port = rdi as u16;
            let val = rsi as u16;
            unsafe {
                let mut pm = x86_64::instructions::port::Port::<u16>::new(port);
                pm.write(val);
            }
            0
        }
        15 => {
            let port = rdi as u16;
            unsafe {
                let mut pm = x86_64::instructions::port::Port::<u32>::new(port);
                pm.read() as u64
            }
        }
        16 => {
            let port = rdi as u16;
            let val = rsi as u32;
            unsafe {
                let mut pm = x86_64::instructions::port::Port::<u32>::new(port);
                pm.write(val);
            }
            0
        }

        100 => {
            let endpoint_id = rdi as usize;
            let msg_ptr = rsi as *const crate::ipc::Message;
            let msg_vaddr = x86_64::VirtAddr::new(rsi);
            if crate::process::validate_user_range(
                msg_vaddr,
                core::mem::size_of::<crate::ipc::Message>(),
            )
            .is_none()
            {
                crate::serial_println!(
                    "[syscall] 100: rejected invalid message pointer {:#X}",
                    rsi
                );
                return u64::MAX;
            }

            let msg = unsafe { &*msg_ptr };
            let endpoint_id = crate::ipc::EndpointId(endpoint_id);

            let mut ipc_state = crate::ipc::IPC.lock();
            if ipc_state.handle_kernel_service(endpoint_id, msg) {
                return 0;
            }
            match ipc_state.send(endpoint_id, *msg) {
                Ok(_) => {
                    drop(ipc_state);
                    crate::process::wake_blocked_on_endpoint(endpoint_id.0);
                    0
                }
                Err(crate::ipc::SendError::InvalidEndpoint) => u64::MAX,
                Err(crate::ipc::SendError::QueueFull(_)) => u64::MAX - 1,
            }
        }
        101 => {
            let endpoint_id = crate::ipc::EndpointId(rdi as usize);
            let msg_ptr = rsi as *mut crate::ipc::Message;
            let msg_vaddr = x86_64::VirtAddr::new(rsi);
            if crate::process::validate_user_range(
                msg_vaddr,
                core::mem::size_of::<crate::ipc::Message>(),
            )
            .is_none()
            {
                crate::serial_println!(
                    "[syscall] 101: rejected invalid message pointer {:#X}",
                    rsi
                );
                return u64::MAX;
            }

            let mut ipc_state = crate::ipc::IPC.lock();
            match ipc_state.recv(endpoint_id) {
                Some(msg) => {
                    unsafe { *msg_ptr = msg };
                    0
                }
                None => {
                    drop(ipc_state);
                    let current = crate::process::CURRENT_PROCESS.load(Ordering::SeqCst);
                    if current != 0 {
                        {
                            let mut table = crate::process::PROCESSES.lock();
                            if let Some(p) = table.get_mut(current) {
                                p.state = crate::process::State::Blocked;
                                p.blocked_on_endpoint = Some(endpoint_id.0);
                            }
                        }
                        unsafe { crate::process::switch_to(current, 0) };
                    }
                    // After waking up, retry the recv
                    let mut ipc_state = crate::ipc::IPC.lock();
                    match ipc_state.recv(endpoint_id) {
                        Some(msg) => {
                            unsafe { *msg_ptr = msg };
                            0
                        }
                        None => u64::MAX,
                    }
                }
            }
        }
        102 => {
            let endpoint_id = crate::ipc::EndpointId(rdi as usize);
            let send_msg_ptr = rsi as *const crate::ipc::Message;
            let recv_msg_ptr = rdx as *mut crate::ipc::Message;
            let send_vaddr = x86_64::VirtAddr::new(rsi);
            let recv_vaddr = x86_64::VirtAddr::new(rdx);
            let msg_size = core::mem::size_of::<crate::ipc::Message>();

            if crate::process::validate_user_range(send_vaddr, msg_size).is_none() {
                crate::serial_println!(
                    "[syscall] 102: rejected invalid send message pointer {:#X}",
                    rsi
                );
                return u64::MAX;
            }
            if crate::process::validate_user_range(recv_vaddr, msg_size).is_none() {
                crate::serial_println!(
                    "[syscall] 102: rejected invalid recv message pointer {:#X}",
                    rdx
                );
                return u64::MAX;
            }

            let send_msg = unsafe { &*send_msg_ptr };

            // Try to handle as a kernel service call first
            {
                let ipc_state = crate::ipc::IPC.lock();
                if let Some(response_msg) = ipc_state.handle_kernel_call(endpoint_id, send_msg) {
                    unsafe { *recv_msg_ptr = response_msg };
                    return 0;
                }
            }

            // Not a kernel service or no response
            {
                let mut ipc_state = crate::ipc::IPC.lock();
                match ipc_state.send(endpoint_id, *send_msg) {
                    Ok(_) => {
                        drop(ipc_state);
                        crate::process::wake_blocked_on_endpoint(endpoint_id.0);
                    }
                    Err(_) => return u64::MAX,
                }
            }
            let mut ipc_state = crate::ipc::IPC.lock();
            match ipc_state.recv(endpoint_id) {
                Some(msg) => {
                    unsafe { *recv_msg_ptr = msg };
                    0
                }
                None => {
                    drop(ipc_state);
                    let current = crate::process::CURRENT_PROCESS.load(Ordering::SeqCst);
                    if current != 0 {
                        {
                            let mut table = crate::process::PROCESSES.lock();
                            if let Some(p) = table.get_mut(current) {
                                p.state = crate::process::State::Blocked;
                                p.blocked_on_endpoint = Some(endpoint_id.0);
                            }
                        }
                        unsafe { crate::process::switch_to(current, 0) };
                    }
                    // After waking up, retry the recv
                    let mut ipc_state = crate::ipc::IPC.lock();
                    match ipc_state.recv(endpoint_id) {
                        Some(msg) => {
                            unsafe { *recv_msg_ptr = msg };
                            0
                        }
                        None => u64::MAX,
                    }
                }
            }
        }
        103 => {
            let endpoint_id = crate::ipc::EndpointId(rdi as usize);
            let current = crate::process::CURRENT_PROCESS.load(Ordering::SeqCst);
            if current == 0 {
                return u64::MAX;
            }
            let mut ipc_state = crate::ipc::IPC.lock();
            match ipc_state
                .registry
                .claim_endpoint(endpoint_id, Some(current))
            {
                Ok(_) => 0,
                Err(_) => u64::MAX,
            }
        }
        104 => {
            let current = crate::process::CURRENT_PROCESS.load(Ordering::SeqCst);
            if current == 0 {
                return u64::MAX;
            }
            let mut ipc_state = crate::ipc::IPC.lock();
            match ipc_state.create_endpoint(Some(current)) {
                Some(id) => id.0 as u64,
                None => u64::MAX,
            }
        }
        105 => {
            let name_vaddr = x86_64::VirtAddr::new(rdi);
            let name_len = rsi as usize;
            if name_len > 64 {
                return u64::MAX;
            }
            if crate::process::validate_user_range(name_vaddr, name_len).is_none() {
                return u64::MAX;
            }
            let name_bytes = unsafe { core::slice::from_raw_parts(rdi as *const u8, name_len) };
            if let Ok(name) = core::str::from_utf8(name_bytes) {
                let initramfs = crate::memory::INITRAMFS.lock();
                if let Some(ramdisk) = *initramfs {
                    let archive = crate::tar::TarArchive::new(ramdisk);
                    for entry in archive.iter() {
                        if entry.name == name && entry.size > 0 {
                            let mut allocator_guard = crate::memory::FRAME_ALLOCATOR.lock();
                            if let Some(allocator) = allocator_guard.as_mut() {
                                let pid = crate::process::spawn(entry.data, allocator);
                                return pid as u64;
                            }
                        }
                    }
                }
            }
            u64::MAX
        }
        _ => 0,
    }
}
