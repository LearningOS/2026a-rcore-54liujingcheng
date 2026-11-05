//! Task management implementation
//!
//! Everything about task management, like starting and switching tasks is
//! implemented here.
//!
//! A single global instance of [`TaskManager`] called `TASK_MANAGER` controls
//! all the tasks in the operating system.
//!
//! Be careful when you see `__switch` ASM function in `switch.S`. Control flow around this function
//! might not be what you expect.

mod context;
mod switch;
#[allow(clippy::module_inception)]
mod task;

use crate::loader::{get_app_data, get_num_app};
use crate::mm::*;
use crate::sync::UPSafeCell;
use crate::trap::TrapContext;
use alloc::vec::Vec;
use lazy_static::*;
use switch::__switch;
pub use task::{TaskControlBlock, TaskStatus};

pub use context::TaskContext;

pub use crate::syscall::{SYSCALL_WRITE, SYSCALL_EXIT, SYSCALL_YIELD, SYSCALL_GET_TIME, SYSCALL_SBRK, SYSCALL_MUNMAP, SYSCALL_MMAP, SYSCALL_TRACE};

pub use crate::mm::translated_byte_buffer;
/// The task manager, where all the tasks are managed.
///
/// Functions implemented on `TaskManager` deals with all task state transitions
/// and task context switching. For convenience, you can find wrappers around it
/// in the module level.
///
/// Most of `TaskManager` are hidden behind the field `inner`, to defer
/// borrowing checks to runtime. You can see examples on how to use `inner` in
/// existing functions on `TaskManager`.
pub struct TaskManager {
    /// total number of tasks
    num_app: usize,
    /// use inner value to get mutable access
    inner: UPSafeCell<TaskManagerInner>,
}

/// The task manager inner in 'UPSafeCell'
struct TaskManagerInner {
    /// task list
    tasks: Vec<TaskControlBlock>,
    /// id of current `Running` task
    current_task: usize,
}

lazy_static! {
    /// a `TaskManager` global instance through lazy_static!
    pub static ref TASK_MANAGER: TaskManager = {
        println!("init TASK_MANAGER");
        let num_app = get_num_app();
        println!("num_app = {}", num_app);
        let mut tasks: Vec<TaskControlBlock> = Vec::new();
        for i in 0..num_app {
            tasks.push(TaskControlBlock::new(get_app_data(i), i));
        }
        TaskManager {
            num_app,
            inner: unsafe {
                UPSafeCell::new(TaskManagerInner {
                    tasks,
                    current_task: 0,
                })
            },
        }
    };
}

impl TaskManager {
    /// Run the first task in task list.
    ///
    /// Generally, the first task in task list is an idle task (we call it zero process later).
    /// But in ch4, we load apps statically, so the first task is a real app.
    fn run_first_task(&self) -> ! {
        let mut inner = self.inner.exclusive_access();
        let next_task = &mut inner.tasks[0];
        next_task.task_status = TaskStatus::Running;
        let next_task_cx_ptr = &next_task.task_cx as *const TaskContext;
        drop(inner);
        let mut _unused = TaskContext::zero_init();
        // before this, we should drop local variables that must be dropped manually
        unsafe {
            __switch(&mut _unused as *mut _, next_task_cx_ptr);
        }
        panic!("unreachable in run_first_task!");
    }

    /// Change the status of current `Running` task into `Ready`.
    fn mark_current_suspended(&self) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_status = TaskStatus::Ready;
    }

    /// Change the status of current `Running` task into `Exited`.
    fn mark_current_exited(&self) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_status = TaskStatus::Exited;
    }

    /// Find next task to run and return task id.
    ///
    /// In this case, we only return the first `Ready` task in task list.
    fn find_next_task(&self) -> Option<usize> {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;
        (current + 1..current + self.num_app + 1)
            .map(|id| id % self.num_app)
            .find(|id| inner.tasks[*id].task_status == TaskStatus::Ready)
    }

    /// Get the current 'Running' task's token.
    pub fn get_current_token(&self) -> usize {
        let inner = self.inner.exclusive_access();
        inner.tasks[inner.current_task].get_user_token()
    }

    /// Get the current 'Running' task's trap contexts.
    fn get_current_trap_cx(&self) -> &'static mut TrapContext {
        let inner = self.inner.exclusive_access();
        inner.tasks[inner.current_task].get_trap_cx()
    }

    /// Change the current 'Running' task's program break
    pub fn change_current_program_brk(&self, size: i32) -> Option<usize> {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].change_program_brk(size)
    }

    /// Switch current `Running` task to the task we have found,
    /// or there is no `Ready` task and we can exit with all applications completed
    fn run_next_task(&self) {
        if let Some(next) = self.find_next_task() {
            let mut inner = self.inner.exclusive_access();
            let current = inner.current_task;
            inner.tasks[next].task_status = TaskStatus::Running;
            inner.current_task = next;
            let current_task_cx_ptr = &mut inner.tasks[current].task_cx as *mut TaskContext;
            let next_task_cx_ptr = &inner.tasks[next].task_cx as *const TaskContext;
            drop(inner);
            // before this, we should drop local variables that must be dropped manually
            unsafe {
                __switch(current_task_cx_ptr, next_task_cx_ptr);
            }
            // go back to user mode
        } else {
            panic!("All applications completed!");
        }
    }
    pub(crate) fn trace_current_syscall(&self, syscall_id : usize) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        let cur_sys =&mut inner.tasks[cur].syscalls;
        match syscall_id {
            SYSCALL_WRITE => cur_sys[0] += 1,
            SYSCALL_EXIT => cur_sys[1] += 1,
            SYSCALL_YIELD => cur_sys[2] += 1,
            SYSCALL_GET_TIME => cur_sys[3] += 1,
            SYSCALL_SBRK => cur_sys[4] += 1,
            SYSCALL_MUNMAP => cur_sys[5] += 1,
            SYSCALL_MMAP => cur_sys[6] += 1,
            SYSCALL_TRACE => cur_sys[7] += 1,
            _ => panic!("Unsupported syscall_id: {}", syscall_id),
        };
        drop(inner);
    }
    pub(crate) fn get_current_syscall(&self, syscall_id : usize) -> isize{
        let inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        let cur_sys = &inner.tasks[cur].syscalls;
        let res = match syscall_id {
            SYSCALL_WRITE => cur_sys[0],
            SYSCALL_EXIT => cur_sys[1],
            SYSCALL_YIELD => cur_sys[2],
            SYSCALL_GET_TIME => cur_sys[3],
            SYSCALL_SBRK => cur_sys[4],
            SYSCALL_MUNMAP => cur_sys[5],
            SYSCALL_MMAP => cur_sys[6],
            SYSCALL_TRACE => cur_sys[7],
            _ => -1,
        };
        drop(inner);
        res
    }
    pub(crate) fn get_buf(&self, ptr: *const u8, len: usize) -> Vec<&'static mut [u8]>{
        translated_byte_buffer(self.get_current_token(), ptr, len)
    }
    pub(crate) fn addto_current_memoryset(&self, start_va: VirtAddr,end_va: VirtAddr, permission: MapPermission){
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].memory_set.insert_framed_area(start_va, end_va, permission);
    }
    pub(crate) fn erase_from_current_memoryset(&self, start_va: VirtAddr, end_va: VirtAddr){
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        let memory_set = &mut inner.tasks[cur].memory_set;
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
}

/// Run the first task in task list.
pub fn run_first_task() {
    TASK_MANAGER.run_first_task();
}

/// Switch current `Running` task to the task we have found,
/// or there is no `Ready` task and we can exit with all applications completed
fn run_next_task() {
    TASK_MANAGER.run_next_task();
}

/// Change the status of current `Running` task into `Ready`.
fn mark_current_suspended() {
    TASK_MANAGER.mark_current_suspended();
}

/// Change the status of current `Running` task into `Exited`.
fn mark_current_exited() {
    TASK_MANAGER.mark_current_exited();
}

/// Suspend the current 'Running' task and run the next task in task list.
pub fn suspend_current_and_run_next() {
    mark_current_suspended();
    run_next_task();
}

/// Exit the current 'Running' task and run the next task in task list.
pub fn exit_current_and_run_next() {
    mark_current_exited();
    run_next_task();
}

/// Get the current 'Running' task's token.
pub fn current_user_token() -> usize {
    TASK_MANAGER.get_current_token()
}

/// Get the current 'Running' task's trap contexts.
pub fn current_trap_cx() -> &'static mut TrapContext {
    TASK_MANAGER.get_current_trap_cx()
}

/// Change the current 'Running' task's program break
pub fn change_program_brk(size: i32) -> Option<usize> {
    TASK_MANAGER.change_current_program_brk(size)
}
