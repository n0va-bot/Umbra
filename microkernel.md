## 1. Multi-process core

### Process table & identity
- [x] `Process` struct (`pid`, `state`, `cr3`, `kernel_stack_top`, `kernel_rsp`, `saved`)
- [x] `Pid::alloc()` (monotonic counter)
- [x] `ProcessTable` with fixed slots (`MAX_PROCESSES = 16`)
- [x] `PROCESSES` global mutex + `current()` / `current_mut()` / `set_current()`
- [x] Use `State::Blocked` / `State::Exited` for real lifecycle management
- [x] Process teardown (free slot, reclaim kernel stack, unmap page tables)

### Per-process kernel stacks
- [x] Static kernel stack pool (`KERNEL_STACK_POOL`, one stack per process slot)
- [x] `allocate_kernel_stack()` — no heap dependency
- [x] Per-process `KERNEL_RSP` wired into syscall entry (replaces global syscall stack)

### Context switch
- [x] `context_switch` naked asm (save/restore callee-saved regs)
- [x] `switch_to(old_idx, new_idx)` — updates TSS RSP0, writes CR3, swaps RSP
- [x] `setup_first_dispatch()` — builds iretq frame on kernel stack for first run
- [x] `return_to_user` — bare `iretq` trampoline

### Userspace entry & return
- [x] ELF loader (`elf_loader.rs`) maps shell into user-accessible pages
- [x] User stack mapped at `0x5555_0000_0000`
- [x] Shell runs in userspace with `syscall` interface
- [x] `syscall 7` (yield) switches back to kernel process (index 0)
- [x] Shell calls `sys_yield()` after each command line
- [x] Return from userspace without a dedicated yield syscall (e.g. on exit)
- [x] Replace the debug `for round in 0..5` loop with a permanent scheduler loop

### Per-process page tables
- [x] Clone boot page tables per process (or fresh PML4 per process)
- [x] Map kernel regions into every address space (higher-half)
- [x] Map user code/stack only into the owning process's CR3
- [x] `process::spawn(elf_bytes)` helper encapsulating map + stack + dispatch setup
- [x] Syscall path validates pointers against current process's mappings

### Scheduler
- [x] Round-robin over `State::Ready` processes in `ProcessTable`
- [x] `schedule()` called from yield syscall and from timer interrupt
- [x] Timer interrupt handler calls `schedule()` for preemptive multitasking
- [x] Kernel thread (index 0) runs the async executor / idle loop when no user work

---

## 2. Collapse syscalls into one IPC primitive

> Replace `syscall.rs`'s `match syscall_nr` with `ipc_send(endpoint_cap, msg)` +
> `ipc_recv(endpoint_cap, msg)` (or `ipc_call = send+recv+reply` in one shot).
> The "syscall number" stops being a thing; a process can only do what its
> capabilities allow.

### Current syscalls (all still numbered)
- [x] `0` — write char / backspace (framebuffer, in-kernel)
- [x] `1` — read scancode (keyboard queue, in-kernel)
- [x] `2` — clear screen (framebuffer, in-kernel)
- [x] `3` — read tick counter
- [x] `4` — power off (ACPI, in-kernel)
- [x] `5` — read RTC (CMOS, in-kernel)
- [x] `6` — PCI scan (in-kernel)
- [x] `7` — yield (process switch)
- [x] `8` — exit (process switch)

### IPC design & implementation
- [x] Define message format (fixed-size buffer, e.g. 64 bytes + tag)
- [x] Kernel endpoint registry (per-process send/receive queues)
- [x] `ipc_send(endpoint_cap, msg)` syscall
- [x] `ipc_recv(endpoint_cap, msg)` syscall (block process until message arrives)
- [x] `ipc_call(endpoint_cap, msg)` — send + wait for reply in one syscall
- [x] Port existing shell operations to IPC calls against in-kernel services
- [ ] Port existing shell operations to IPC calls against userspace servers
- [x] Remove the `match syscall_nr` dispatch table (only 7 and 8 remain)

---

## 3. Move drivers out of kernel

> `framebuffer` → `fb-server`, `cmos` → `rtc`, `pci` → `pci-arbiter`,
> `acpi::power_off` → power service. Kernel only exposes mapping / port-I/O
> capabilities.

### Framebuffer → userspace `fb-server`
- [x] `mmap_framebuffer()` — map physical FB region into caller's page table
- [x] Extract `framebuffer.rs` rendering into userspace binary
- [x] Shell talks to fb-server via IPC (write, clear, backspace) (in-kernel for now)
- [x] Remove direct framebuffer access from syscall 0/2

### CMOS / RTC → userspace `rtc`
- [x] Port-I/O capability for ports `0x70`/`0x71`
- [x] Move `cmos.rs` logic into userspace `rtc` process
- [x] Shell `date` command → IPC to rtc server (in-kernel for now)
- [x] Remove syscall 5

### PCI → userspace `pci-arbiter`
- [x] Port-I/O capability for `0xCF8`/`0xCFC`
- [x] Move `pci.rs` bus walk into userspace
- [x] Shell `lspci` → IPC to pci-arbiter (in-kernel for now)
- [x] Remove syscall 6
- [ ] Deferred — Hand out per-device capabilities to requesters (requires driver model)

### ACPI power → userspace power service
- [x] Port-I/O cap for PM1a control register
- [x] Move `acpi::power_off` into userspace
- [x] Shell `poweroff` → IPC to power service (in-kernel for now)
- [x] Remove syscall 4

### Keyboard → userspace `ps2-kbd` (or keep in-kernel short-term)
- [x] Move scancode IRQ handler + queue behind a capability
- [x] Userspace keyboard server reads port `0x60` via port cap
- [x] Shell reads keys via IPC instead of syscall 1 (in-kernel for now)
- [x] Remove syscall 1
- [x] (Optional) keep IRQ demux in kernel, deliver scancodes via IPC

### Serial
- [x] Decision: keep in-kernel for boot logs.
- [x] Currently in-kernel (`serial.rs`) — fine for now

---

## 4. Capabilities

> Each process has a small CSpace (capability table). To do port I/O, you need a
> `Port` cap; to map a physical region, a `Frame` cap; to talk to another
> process, an `Endpoint` cap.

- [x] `Cap` type enum (`Endpoint`, `Port`, `Frame`, `Process`, …)
- [x] Per-process capability table (CSpace) with slot allocation
- [ ] Capability derivation / delegation on `process::spawn`
- [ ] Syscall argument validation: IPC only if caller holds `Endpoint` cap
- [x] Port-I/O syscall gated on `Port` cap (range-checked)
- [ ] `mmap` syscall gated on `Frame` cap
- [ ] Bootstrap: shell starts with caps to fb-server, rtc, pci-arbiter, power, keyboard — nothing else

---

## 5. Split the kernel into workspace crates

> `kernel-arch`, `kernel-core`, `kernel-hal`, thin `kernel` binary,
> `userspace/libumbra`, per-service userspace binaries.

- [x] Workspace with `kernel`, `userspace`, `builder` crates
- [x] `kernel-arch` — GDT, IDT, syscall entry, interrupts, serial, memory, acpi, cmos, pci, userspace
- [x] `kernel-core` — process table, scheduler, IPC, capability table, allocator, elf_loader, syscall logic, tar, task
- [x] `kernel-hal` — minimal traits placeholder (clock, frame allocator)
- [x] `kernel` — thin binary wiring boot only, re-exports from kernel-arch and kernel-core
- [x] `userspace/libumbra` — shared library with `cap`, `ipc`, `process` modules (placeholder)
- [x] `userspace/{shell,fb-server,rtc,pci-arbiter,ps2-kbd,power,SerV,tick-server}` — per-service binaries exist

---

## 6. Clean the boot path

> `arch::init()` → `mem::init()` → `ipc::init()` → `process::init()`.
> No inline `mapper.map_to` for user stack in `kernel_main`.

- [x] `umbra::init()` — GDT, IDT, PIC, enable interrupts
- [x] Split into `arch::init()`, `mem::init(boot_info)`, `ipc::init()`, `process::init()`
- [x] Move user stack mapping into `process::spawn`
- [x] `kernel_main` becomes a short init chain + `schedule()` forever
- [x] Remove inline ELF load / stack map / switch_to boilerplate from `main.rs`
