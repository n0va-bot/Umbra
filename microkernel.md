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
- [/] Define message format (fixed-size buffer, e.g. 64 bytes + tag)
- [ ] Kernel endpoint registry (per-process send/receive queues)
- [ ] `ipc_send(endpoint_cap, msg)` syscall
- [ ] `ipc_recv(endpoint_cap, msg)` syscall (block process until message arrives)
- [ ] `ipc_call(endpoint_cap, msg)` — send + wait for reply in one syscall
- [ ] Port existing shell operations to IPC calls against userspace servers
- [ ] Remove the `match syscall_nr` dispatch table

---

## 3. Move drivers out of kernel

> `framebuffer` → `fb-server`, `cmos` → `rtc`, `pci` → `pci-arbiter`,
> `acpi::power_off` → power service. Kernel only exposes mapping / port-I/O
> capabilities.

### Framebuffer → userspace `fb-server`
- [ ] `mmap_framebuffer()` — map physical FB region into caller's page table
- [ ] Extract `framebuffer.rs` rendering into userspace binary
- [ ] Shell talks to fb-server via IPC (write, clear, backspace)
- [ ] Remove direct framebuffer access from syscall 0/2

### CMOS / RTC → userspace `rtc`
- [ ] Port-I/O capability for ports `0x70`/`0x71`
- [ ] Move `cmos.rs` logic into userspace `rtc` process
- [ ] Shell `date` command → IPC to rtc server
- [ ] Remove syscall 5

### PCI → userspace `pci-arbiter`
- [ ] Port-I/O capability for `0xCF8`/`0xCFC`
- [ ] Move `pci.rs` bus walk into userspace
- [ ] Hand out per-device capabilities to requesters
- [ ] Shell `lspci` → IPC to pci-arbiter
- [ ] Remove syscall 6

### ACPI power → userspace power service
- [ ] Port-I/O cap for PM1a control register
- [ ] Move `acpi::power_off` into userspace
- [ ] Shell `poweroff` → IPC to power service
- [ ] Remove syscall 4

### Keyboard → userspace `ps2-kbd` (or keep in-kernel short-term)
- [ ] Move scancode IRQ handler + queue behind a capability
- [ ] Userspace keyboard server reads port `0x60` via port cap
- [ ] Shell reads keys via IPC instead of syscall 1
- [ ] (Optional) keep IRQ demux in kernel, deliver scancodes via IPC

### Serial
- [ ] Decision: keep in-kernel for boot logs (defensible) or move to `tty` server
- [x] Currently in-kernel (`serial.rs`) — fine for now

---

## 4. Capabilities

> Each process has a small CSpace (capability table). To do port I/O, you need a
> `Port` cap; to map a physical region, a `Frame` cap; to talk to another
> process, an `Endpoint` cap.

- [ ] `Cap` type enum (`Endpoint`, `Port`, `Frame`, `Process`, …)
- [ ] Per-process capability table (CSpace) with slot allocation
- [ ] Capability derivation / delegation on `process::spawn`
- [ ] Syscall argument validation: IPC only if caller holds `Endpoint` cap
- [ ] Port-I/O syscall gated on `Port` cap (range-checked)
- [ ] `mmap` syscall gated on `Frame` cap
- [ ] Bootstrap: shell starts with caps to fb-server, rtc, pci-arbiter, power, keyboard — nothing else

---

## 5. Split the kernel into workspace crates

> `kernel-arch`, `kernel-core`, `kernel-hal`, thin `kernel` binary,
> `userspace/libumbra`, per-service userspace binaries.

- [x] Workspace with `kernel`, `userspace`, `builder` crates
- [ ] `kernel-arch` (or `kernel/arch/x86_64`) — GDT, IDT, syscall entry, context switch, port-I/O traps
- [ ] `kernel-core` — process table, scheduler, IPC, capability table, page-table management
- [ ] `kernel-hal` — minimal traits (clock, frame allocator)
- [ ] `kernel` — thin binary wiring boot only
- [ ] `userspace/libumbra` — shared `cap`, `ipc`, `process::spawn` helpers
- [ ] `userspace/{shell,fb-server,rtc,pci-arbiter,ps2-kbd,power}` — one binary each

---

## 6. Clean the boot path

> `arch::init()` → `mem::init()` → `ipc::init()` → `process::init()`.
> No inline `mapper.map_to` for user stack in `kernel_main`.

- [x] `umbra::init()` — GDT, IDT, PIC, enable interrupts
- [ ] Split into `arch::init()`, `mem::init(boot_info)`, `ipc::init()`, `process::init()`
- [x] Move user stack mapping into `process::spawn`
- [x] `kernel_main` becomes a short init chain + `schedule()` forever
- [x] Remove inline ELF load / stack map / switch_to boilerplate from `main.rs`
