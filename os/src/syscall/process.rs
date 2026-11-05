//! Process management syscalls
use crate::config::PAGE_SIZE;
use crate::task::{change_program_brk, exit_current_and_run_next, suspend_current_and_run_next};

use crate::mm::*;

use crate::{
    task::TASK_MANAGER,
    timer::get_time_us,
};
#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    let tmp = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    unsafe{
        let mut _vec = TASK_MANAGER.get_buf(_ts as *const u8, 16);
        if _vec.len() == 1 {
            _vec[0].as_mut_ptr().copy_from(&tmp as *const TimeVal as *const u8, 16);
        }else {
            _vec[0].as_mut_ptr().copy_from(&(tmp.sec) as *const usize as *const u8, 8);
            _vec[1].as_mut_ptr().copy_from(&(tmp.usec) as *const usize as *const u8, 8);
        }
    }
    0
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(_trace_request: usize, _id: usize, _data: usize) -> isize {
    trace!("kernel: sys_trace");
    match _trace_request {
        0 => {
            let pte : PageTableEntry;
            let _pte = PageTable::from_token(TASK_MANAGER.get_current_token()).translate(VirtAddr::from(_id).floor());
            if let Option::None = _pte {
                return -1;
            }else {
                pte = _pte.unwrap();
            }
            if pte.flags() & PTEFlags::V == PTEFlags::empty() || pte.flags() & PTEFlags::U == PTEFlags::empty() || pte.flags() & PTEFlags::R == PTEFlags::empty(){
                return -1;
            }
            let buffers = TASK_MANAGER.get_buf(_id as *const u8, 1);
            buffers[0][0] as isize
        },
        1 => {
            let data: u8 = _data as u8;
            let pte : PageTableEntry;
            let _pte = PageTable::from_token(TASK_MANAGER.get_current_token()).translate(VirtAddr::from(_id).floor());
            if let Option::None = _pte {
                return -1;
            }else {
                pte = _pte.unwrap();
            }
            if pte.flags() & PTEFlags::V == PTEFlags::empty() || pte.flags() & PTEFlags::U == PTEFlags::empty() || pte.flags() & PTEFlags::W == PTEFlags::empty(){
                return -1;
            }
            let mut buffers = TASK_MANAGER.get_buf(_id as *const u8, 1);
            buffers[0][0] = data;
            0
        },
        2 => {
            TASK_MANAGER.get_current_syscall(_id)
        },
        _ => {
            -1
        }
    }
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(_start: usize, _len: usize, _prot: usize) -> isize {
    trace!("kernel: sys_mmap NOT IMPLEMENTED YET!");
    if _start % PAGE_SIZE != 0 || _prot & !0x7 != 0 || _prot & 0x7 == 0{
        return -1;
    }
    let mut curvad = _start;
    let _end = _start + ((_len + PAGE_SIZE - 1) & !(PAGE_SIZE-1));
    while curvad < _end{
        let pte = PageTable::from_token(TASK_MANAGER.get_current_token()).translate(VirtAddr::from(curvad).floor());
        if let Option::Some(_pte) = pte{
            if _pte.is_valid() {
                return -1;
            }
        }
        curvad += PAGE_SIZE;
    }
    let mut mp = MapPermission::empty();
    mp |= MapPermission::U;
    if _prot & 1 != 0 {
        mp |= MapPermission::R;
    }
    if (_prot >> 1) & 1 != 0 {
        mp |= MapPermission::W;
    }
    if (_prot >> 2) & 1 != 0 {
        mp |= MapPermission::X;
    }
    TASK_MANAGER.addto_current_memoryset(VirtAddr::from(_start), VirtAddr::from(_end), mp);
    0
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    trace!("kernel: sys_munmap NOT IMPLEMENTED YET!");
    if _start % PAGE_SIZE != 0{
        return -1;
    }
    let mut curvad = _start;
    let _end = _start + ((_len + PAGE_SIZE - 1) & !(PAGE_SIZE-1));
    while curvad < _end{
        let pte = PageTable::from_token(TASK_MANAGER.get_current_token()).translate(VirtAddr::from(curvad).floor());
        if let Option::Some(_pte) = pte{
            if !_pte.is_valid() {
                return -1;
            }
        }else{
            return -1;
        }
        curvad += PAGE_SIZE;
    }
    TASK_MANAGER.erase_from_current_memoryset(VirtAddr::from(_start), VirtAddr::from(_end));
    0
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
