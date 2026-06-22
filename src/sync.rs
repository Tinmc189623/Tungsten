// sync.rs — Tungsten 内核同步原语
// SpinLock（关中断测试-设置锁）、Mutex（阻塞互斥）、RwLock（读写锁）、Semaphore（计数信号量）
// Copyright (C) 2026 Nexsteaduser. All rights reserved.
// SPDX-License-Identifier: GPL-3.0-or-later



use core::cell::UnsafeCell;
use core::fmt;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/* ══════════════════════════════════════════════
   SpinLock — 测试-设置自旋锁（关中断变体可选）
   ══════════════════════════════════════════════ */

/// 基于 test-and-set 的自旋锁，保护内部数据 `T`。
///
/// 采用 TAS (test-and-set) + TTAS (test-and-test-and-set) 策略减少总线争用。
/// 此锁不关闭中断；如需在中断上下文中使用，请配合 `IrqSaveSpinlock`。
pub struct SpinLock<T> {
    lock: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for SpinLock<T> {}
unsafe impl<T: Send> Send for SpinLock<T> {}

/// `SpinLock` 的 RAII 守卫，释放时自动解锁。
pub struct SpinLockGuard<'a, T> {
    lock: &'a AtomicBool,
    data: &'a mut T,
}

impl<T> SpinLock<T> {
    /// 创建新的未锁定自旋锁，包裹给定数据。
    pub const fn new(data: T) -> Self {
        SpinLock {
            lock: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    /// 获取自旋锁，忙等待直到锁可用。
    ///
    /// 使用 TTAS 策略：先 Relaxed 读检测锁状态，减少缓存一致性流量，
    /// 检测到可能可用后再用 Acquire-swap 尝试获取。
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        while self.lock.swap(true, Ordering::Acquire) {
            while self.lock.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
        SpinLockGuard {
            lock: &self.lock,
            data: unsafe { &mut *self.data.get() },
        }
    }

    /// 尝试获取自旋锁，立即返回。成功返回 `Some(guard)`，失败返回 `None`。
    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        if self.lock.swap(true, Ordering::Acquire) {
            None
        } else {
            Some(SpinLockGuard {
                lock: &self.lock,
                data: unsafe { &mut *self.data.get() },
            })
        }
    }

    /// 返回锁当前是否被持有（仅供调试，非同步手段）。
    pub fn is_locked(&self) -> bool {
        self.lock.load(Ordering::Relaxed)
    }
}

impl<T: fmt::Debug> fmt::Debug for SpinLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpinLock")
            .field("locked", &self.lock.load(Ordering::Relaxed))
            .finish()
    }
}

impl<'a, T> Deref for SpinLockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.data
    }
}

impl<'a, T> DerefMut for SpinLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.data
    }
}

impl<'a, T> Drop for SpinLockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.store(false, Ordering::Release);
    }
}

/* ══════════════════════════════════════════════
   IrqSaveSpinlock — 关中断自旋锁（中断上下文安全）
   ══════════════════════════════════════════════ */

/// 关中断自旋锁：获取锁前先保存 RFLAGS 并执行 CLI，
/// 释放锁后根据保存的 IF 位恢复中断状态。
///
/// 用于中断处理程序与正常代码共享数据的场景。
pub struct IrqSaveSpinlock<T> {
    lock: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for IrqSaveSpinlock<T> {}
unsafe impl<T: Send> Send for IrqSaveSpinlock<T> {}

/// `IrqSaveSpinlock` 的 RAII 守卫，释放时恢复中断状态。
pub struct IrqSaveGuard<'a, T> {
    lock: &'a AtomicBool,
    data: &'a mut T,
    /// 保存的 RFLAGS 值，用于判断释放时是否需要重新开中断
    irq_state: u64,
}

impl<T> IrqSaveSpinlock<T> {
    /// 创建新的关中断自旋锁。
    pub const fn new(data: T) -> Self {
        IrqSaveSpinlock {
            lock: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    /// 获取锁并关闭中断。保存当前 RFLAGS 以便后续恢复。
    pub fn lock(&self) -> IrqSaveGuard<'_, T> {
        let irq_state: u64;
        unsafe {
            core::arch::asm!("pushfq; pop {}", out(reg) irq_state);
            core::arch::asm!("cli");
        }
        while self.lock.swap(true, Ordering::Acquire) {
            while self.lock.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
        IrqSaveGuard {
            lock: &self.lock,
            data: unsafe { &mut *self.data.get() },
            irq_state,
        }
    }
}

impl<'a, T> Deref for IrqSaveGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.data
    }
}

impl<'a, T> DerefMut for IrqSaveGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.data
    }
}

impl<'a, T> Drop for IrqSaveGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.store(false, Ordering::Release);
        // 仅当获取前中断是开启状态时才恢复
        if self.irq_state & 0x200 != 0 {
            unsafe {
                core::arch::asm!("sti");
            }
        }
    }
}

/* ══════════════════════════════════════════════
   Mutex — 阻塞互斥锁（调度器就绪后升级为睡眠等待）
   ══════════════════════════════════════════════ */

/// 阻塞互斥锁。
///
/// 当前实现基于自旋等待。调度器完全就绪后可升级为睡眠/唤醒机制，
/// 将等待任务加入队列并调用 `yield_now()` 让出 CPU。
///
/// 与 `SpinLock` 的区别在于语义层面：Mutex 用于可能长时间持锁的场景，
/// 未来实现中等待者不会消耗 CPU 周期。
pub struct Mutex<T> {
    inner: SpinLock<T>,
}

/// `Mutex` 的 RAII 守卫。
pub struct MutexGuard<'a, T> {
    guard: SpinLockGuard<'a, T>,
}

impl<T> Mutex<T> {
    /// 创建新的互斥锁。
    pub const fn new(data: T) -> Self {
        Mutex {
            inner: SpinLock::new(data),
        }
    }

    /// 获取互斥锁，阻塞直到成功。
    pub fn lock(&self) -> MutexGuard<'_, T> {
        MutexGuard {
            guard: self.inner.lock(),
        }
    }

    /// 尝试获取互斥锁，不阻塞。
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        self.inner
            .try_lock()
            .map(|guard| MutexGuard { guard })
    }
}

impl<'a, T> Deref for MutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.guard.deref()
    }
}

impl<'a, T> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.guard.deref_mut()
    }
}

/* ══════════════════════════════════════════════
   RwLock — 读写锁（多读单写）
   ══════════════════════════════════════════════ */

/// 读写锁：允许多个并发读者或单个独占写者。
///
/// 读者之间不互斥，写者排斥所有读者和其他写者。
/// 写者优先级高于读者以避免写饥饿。
pub struct RwLock<T> {
    /// 写锁标志
    lock: AtomicBool,
    /// 活跃读者计数（UnsafeCell 因为读者需要在持有共享引用时修改计数）
    readers: UnsafeCell<u32>,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for RwLock<T> {}
unsafe impl<T: Send> Send for RwLock<T> {}

/// 读锁守卫，释放时递减读者计数。
pub struct ReadGuard<'a, T> {
    lock: &'a AtomicBool,
    readers: &'a UnsafeCell<u32>,
    data: &'a T,
}

/// 写锁守卫，释放时清除写锁标志。
pub struct WriteGuard<'a, T> {
    lock: &'a AtomicBool,
    data: &'a mut T,
}

impl<T> RwLock<T> {
    /// 创建新的读写锁。
    pub const fn new(data: T) -> Self {
        RwLock {
            lock: AtomicBool::new(false),
            readers: UnsafeCell::new(0),
            data: UnsafeCell::new(data),
        }
    }

    /// 获取读锁。如果有写者持有锁则自旋等待。
    ///
    /// 多个读者可同时持有读锁。增加读者计数后会再次检查写锁，
    /// 若写锁在此期间被获取则回退计数并重新等待。
    pub fn read(&self) -> ReadGuard<'_, T> {
        loop {
            // 等待写锁释放
            while self.lock.load(Ordering::Acquire) {
                core::hint::spin_loop();
            }
            // 增加读者计数
            unsafe {
                *self.readers.get() += 1;
            }
            // 双重检查：增加计数后写锁是否被获取
            if self.lock.load(Ordering::Acquire) {
                unsafe {
                    *self.readers.get() -= 1;
                }
                continue;
            }
            break;
        }
        ReadGuard {
            lock: &self.lock,
            readers: &self.readers,
            data: unsafe { &*self.data.get() },
        }
    }

    /// 获取写锁。排斥所有读者和其他写者。
    ///
    /// 先获取写锁标志，然后等待所有活跃读者退出。
    pub fn write(&self) -> WriteGuard<'_, T> {
        while self.lock.swap(true, Ordering::Acquire) {
            while self.lock.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
        // 等待所有读者离开临界区
        while unsafe { *self.readers.get() } > 0 {
            core::hint::spin_loop();
        }
        WriteGuard {
            lock: &self.lock,
            data: unsafe { &mut *self.data.get() },
        }
    }
}

impl<'a, T> Deref for ReadGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.data
    }
}

impl<'a, T> Drop for ReadGuard<'a, T> {
    fn drop(&mut self) {
        unsafe {
            *self.readers.get() -= 1;
        }
    }
}

impl<'a, T> Deref for WriteGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.data
    }
}

impl<'a, T> DerefMut for WriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.data
    }
}

impl<'a, T> Drop for WriteGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.store(false, Ordering::Release);
    }
}

/* ── 兼容别名（现有代码使用旧命名） ── */

/// 兼容别名，供现有模块使用旧名称 `Spinlock` 引用。
pub type Spinlock<T> = SpinLock<T>;

/* ══════════════════════════════════════════════
   Semaphore — 计数信号量
   ══════════════════════════════════════════════ */

/// 计数信号量，用于控制同时访问某资源的并发数量。
///
/// `post()` (V 操作) 递增计数并唤醒等待者；
/// `wait()` (P 操作) 尝试递减计数，计数为零时自旋等待。
///
/// 典型用途：限制同时进入临界区的任务数量、生产者-消费者同步。
pub struct Semaphore {
    count: AtomicU64,
}

impl Semaphore {
    /// 创建初始计数为 `initial` 的信号量。
    pub const fn new(initial: u64) -> Self {
        Semaphore {
            count: AtomicU64::new(initial),
        }
    }

    /// P 操作（wait / down）：尝试获取一个许可。
    ///
    /// 若计数大于零则原子递减并返回；否则自旋等待直到有许可可用。
    pub fn wait(&self) {
        loop {
            let current = self.count.load(Ordering::Acquire);
            if current > 0 {
                if self
                    .count
                    .compare_exchange_weak(
                        current,
                        current - 1,
                        Ordering::AcqRel,
                        Ordering::Relaxed,
                    )
                    .is_ok()
                {
                    return;
                }
            }
            core::hint::spin_loop();
        }
    }

    /// 尝试 P 操作，不阻塞。成功返回 `true`，无许可时返回 `false`。
    pub fn try_wait(&self) -> bool {
        let current = self.count.load(Ordering::Acquire);
        if current > 0 {
            self.count
                .compare_exchange(current, current - 1, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
        } else {
            false
        }
    }

    /// V 操作（post / signal / up）：释放一个许可，可能唤醒一个等待者。
    pub fn post(&self) {
        self.count.fetch_add(1, Ordering::Release);
    }

    /// 返回当前可用许可数（仅供调试，非同步手段）。
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }
}
