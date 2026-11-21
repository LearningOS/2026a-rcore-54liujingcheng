//! Task management implementation
//!
//! Everything about task management, like starting and switching tasks is
//! implemented here.
//!
//! A single global instance of [`TaskManager`] called `TASK_MANAGER` controls
//! all the tasks in the whole operating system.
//!
//! A single global instance of [`Processor`] called `PROCESSOR` monitors running
//! task(s) for each core.
//!
//! A single global instance of `PID_ALLOCATOR` allocates pid for user apps.
//!
//! Be careful when you see `__switch` ASM function in `switch.S`. Control flow around this function
//! might not be what you expect.
mod context;
mod id;
mod manager;
mod processor;
mod switch;
#[allow(clippy::module_inception)]
mod task;
use alloc::vec::Vec;
pub use crate::loader::get_app_data_by_name;
pub use alloc::sync::Arc;
pub use lazy_static::*;
pub use manager::{fetch_task, TaskManager, TASK_MANAGER};
pub use switch::__switch;
pub use task::{TaskControlBlock, TaskStatus};
pub use crate::syscall::{SYSCALL_WRITE, SYSCALL_EXIT, SYSCALL_YIELD, SYSCALL_GET_TIME, SYSCALL_SBRK, SYSCALL_MUNMAP, SYSCALL_MMAP};

pub use crate::mm::{translated_byte_buffer, VirtAddr, MapPermission, MapArea, MapType, VPNRange};
pub use context::TaskContext;
pub use id::{kstack_alloc, pid_alloc, KernelStack, PidHandle};
pub use manager::add_task;
pub use processor::{
    current_task, current_trap_cx, current_user_token, run_tasks, schedule, take_current_task,
    Processor,
};
/// Suspend the current 'Running' task and run the next task in task list.
pub fn suspend_current_and_run_next() {
    // There must be an application running.
    let task = take_current_task().unwrap();

    // ---- access current TCB exclusively
    let mut task_inner = task.inner_exclusive_access();
    let task_cx_ptr = &mut task_inner.task_cx as *mut TaskContext;
    // Change status to Ready
    task_inner.task_status = TaskStatus::Ready;
    drop(task_inner);
    // ---- release current PCB

    // push back to ready queue.
    add_task(task);
    // jump to scheduling cycle
    schedule(task_cx_ptr);
}

/// pid of usertests app in make run TEST=1
pub const IDLE_PID: usize = 0;

/// Exit the current 'Running' task and run the next task in task list.
pub fn exit_current_and_run_next(exit_code: i32) {
    // take from Processor
    let task = take_current_task().unwrap();

    let pid = task.getpid();
    if pid == IDLE_PID {
        println!(
            "[kernel] Idle process exit with exit_code {} ...",
            exit_code
        );
        panic!("All applications completed!");
    }

    // **** access current TCB exclusively
    let mut inner = task.inner_exclusive_access();
    // Change status to Zombie
    inner.task_status = TaskStatus::Zombie;
    // Record exit code
    inner.exit_code = exit_code;
    // do not move to its parent but under initproc

    // ++++++ access initproc TCB exclusively
    {
        let mut initproc_inner = INITPROC.inner_exclusive_access();
        for child in inner.children.iter() {
            child.inner_exclusive_access().parent = Some(Arc::downgrade(&INITPROC));
            initproc_inner.children.push(child.clone());
        }
    }
    // ++++++ release parent PCB

    inner.children.clear();
    // deallocate user space
    inner.memory_set.recycle_data_pages();
    drop(inner);
    // **** release current PCB
    // drop task manually to maintain rc correctly
    drop(task);
    // we do not have to save task context
    let mut _unused = TaskContext::zero_init();
    schedule(&mut _unused as *mut _);
}
pub(crate) fn get_current_buf(ptr: *const u8, len: usize) -> Vec<&'static mut [u8]>{
    translated_byte_buffer(current_user_token(), ptr, len)
}
pub(crate) fn addto_current_memoryset(start_va: VirtAddr,end_va: VirtAddr, permission: MapPermission){
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    inner.memory_set.insert_framed_area(start_va, end_va, permission);
}
pub(crate) fn erase_from_current_memoryset(start_va: VirtAddr, end_va: VirtAddr){
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    let memory_set = &mut inner.memory_set;
    let area = &mut memory_set.areas;
    let pgtable = &mut memory_set.page_table;
    let mut i = 0;
    while i < area.len() {
        let l = area[i].vpn_range.get_start();
        let r = area[i].vpn_range.get_end();
        if l >= end_va.ceil() || r <= start_va.floor(){
            i += 1;
            continue;
        }
        if l < start_va.floor() && r > end_va.ceil() {
            for x in area[i].vpn_range{
                if x < start_va.floor() {
                    continue;
                }else if x < end_va.ceil() {
                    area[i].unmap_one(pgtable, x);
                }else {
                    break;
                }
            }
            let vrange1 = VPNRange::new(l, start_va.floor());
            area[i].vpn_range = vrange1;
            let perm = area[i].map_perm;
            let frames = area[i].data_frames.split_off(&end_va.ceil());
            let vrange2 = VPNRange::new(end_va.ceil(), r);
            let newarea = MapArea{vpn_range : vrange2,data_frames : frames,map_type : MapType::Framed,map_perm : perm};
            area.push(newarea);
            return ;
        }else if l >= start_va.floor() && r <= end_va.ceil(){
            area[i].unmap(pgtable);
            area.remove(i);
            continue;
        }else {
            if r > end_va.ceil(){
                for x in area[i].vpn_range {
                    if x < end_va.ceil() {
                        area[i].unmap_one(pgtable, x);
                    }
                    else{
                        break;
                    }
                }
                let vrange = VPNRange::new(end_va.ceil(), r);
                area[i].vpn_range = vrange;
                i += 1;
                continue;
            }
            if l < start_va.floor(){
                area[i].shrink_to(pgtable, start_va.ceil());
                i += 1;
                continue;
            }
        }
    }
}
lazy_static! {
    /// Creation of initial process
    ///
    /// the name "initproc" may be changed to any other app name like "usertests",
    /// but we have user_shell, so we don't need to change it.
    pub static ref INITPROC: Arc<TaskControlBlock> = Arc::new(TaskControlBlock::new(
        get_app_data_by_name("ch5b_initproc").unwrap()
    ));
}

///Add init process to the manager
pub fn add_initproc() {
    add_task(INITPROC.clone());
}
