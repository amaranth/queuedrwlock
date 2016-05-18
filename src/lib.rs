// Copyright (c) 2016 Travis Watkins
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

extern crate poison;

use std::{fmt, mem};
use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::sync::{LockResult, TryLockError, TryLockResult};

use poison::{Poison, PoisonGuard};
use raw::RawQueuedRwLock;

mod raw;

/// RwLock that implements a FIFO queue for the write lock via ticket locks
pub struct QueuedRwLock<T> {
    inner: RawQueuedRwLock,
    data: UnsafeCell<Poison<T>>
}

unsafe impl<T: Send> Send for QueuedRwLock<T> {}
unsafe impl<T: Sync> Sync for QueuedRwLock<T> {}

impl<T> QueuedRwLock<T> {
    pub fn new(data: T) -> QueuedRwLock<T> {
        QueuedRwLock {
            inner: RawQueuedRwLock::new(),
            data: UnsafeCell::new(Poison::new(data)),
        }
    }

    pub fn read(&self) -> LockResult<QueuedRwLockReadGuard<T>> {
        self.inner.read();
        unsafe { QueuedRwLockReadGuard::new(self) }
    }

    pub fn try_read(&self) -> TryLockResult<QueuedRwLockReadGuard<T>> {
        if self.inner.try_read() {
            Ok(try!(unsafe { QueuedRwLockReadGuard::new(self) }))
        } else {
            Err(TryLockError::WouldBlock)
        }
    }

    pub fn take_ticket(&self) -> QueuedRwLockTicketGuard<T> {
        let ticket = self.inner.take_ticket();
        QueuedRwLockTicketGuard::new(self, ticket)
    }

    pub fn write(&self) -> LockResult<QueuedRwLockWriteGuard<T>> {
        let ticket = self.take_ticket();
        ticket.write()
    }

    pub fn try_write(&self) -> TryLockResult<QueuedRwLockWriteGuard<T>> {
        if self.inner.try_write_skip_queue() {
            // dummy ticket for write guard
            let ticket = QueuedRwLockTicketGuard::new(self, 0);
            Ok(try!(unsafe { QueuedRwLockWriteGuard::new(ticket) }))
        } else {
            Err(TryLockError::WouldBlock)
        }
    }

    pub fn into_inner(self) -> LockResult<T> {
        unsafe { self.data.into_inner().into_inner() }
    }

    pub fn get_mut(&mut self) -> LockResult<&mut T> {
        poison::map_result(unsafe { &mut *self.data.get() }.lock(), |guard| {
            unsafe { guard.into_mut() }
        })
    }
}

impl<T: fmt::Debug> fmt::Debug for QueuedRwLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.try_read() {
            Ok(guard) => write!(f, "QueuedRwLock {{ data: {:?} }}", &*guard),
            Err(TryLockError::Poisoned(err)) => {
                write!(f, "QueuedRwLock {{ data: Poisoned({:?}) }}", &**err.get_ref())
            },
            Err(TryLockError::WouldBlock) => write!(f, "QueuedRwLock {{ <locked> }}")
        }
    }
}

#[must_use]
pub struct QueuedRwLockReadGuard<'a, T: 'a> {
    lock: &'a QueuedRwLock<T>,
    data: &'a T,
}

impl<'a, T> QueuedRwLockReadGuard<'a, T> {
    unsafe fn new(lock: &'a QueuedRwLock<T>) -> LockResult<QueuedRwLockReadGuard<'a, T>> {
        poison::map_result((*lock.data.get()).get(), |data| {
            QueuedRwLockReadGuard {
                lock: lock,
                data: data,
            }
        })
    }
}

unsafe impl<'a, T: Send> Send for QueuedRwLockReadGuard<'a, T> {}
unsafe impl<'a, T: Sync> Sync for QueuedRwLockReadGuard<'a, T> {}

impl<'a, T> Deref for QueuedRwLockReadGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T { self.data }
}

impl<'a, T> Drop for QueuedRwLockReadGuard<'a, T> {
    fn drop(&mut self) { self.lock.inner.read_unlock() }
}

#[must_use]
pub struct QueuedRwLockWriteGuard<'a, T: 'a> {
    lock: &'a QueuedRwLock<T>,
    data: PoisonGuard<'a, T>,
}

impl<'a, T> QueuedRwLockWriteGuard<'a, T> {
    unsafe fn new(ticket: QueuedRwLockTicketGuard<'a, T>) -> LockResult<QueuedRwLockWriteGuard<'a, T>> {
        let result = poison::map_result((*ticket.lock.data.get()).lock(), |data| {
            QueuedRwLockWriteGuard {
                lock: ticket.lock,
                data: data,
            }
        });

        // Make sure we don't double lock
        mem::forget(ticket);
        result
    }
}

unsafe impl<'a, T: Send> Send for QueuedRwLockWriteGuard<'a, T> {}
unsafe impl<'a, T: Sync> Sync for QueuedRwLockWriteGuard<'a, T> {}

impl<'a, T> Deref for QueuedRwLockWriteGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T { self.data.get() }
}

impl<'a, T> DerefMut for QueuedRwLockWriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T { self.data.get_mut() }
}

impl<'a, T> Drop for QueuedRwLockWriteGuard<'a, T> {
    fn drop(&mut self) { self.lock.inner.write_unlock() }
}

#[must_use]
pub struct QueuedRwLockTicketGuard<'a, T: 'a> {
    lock: &'a QueuedRwLock<T>,
    ticket: usize,
}

impl<'a, T> QueuedRwLockTicketGuard<'a, T> {
    fn new(lock: &'a QueuedRwLock<T>, ticket: usize) -> QueuedRwLockTicketGuard<'a, T> {
        QueuedRwLockTicketGuard {
            lock: lock,
            ticket: ticket,
        }
    }

    pub fn write(self) -> LockResult<QueuedRwLockWriteGuard<'a, T>> {
        self.lock.inner.write(self.ticket);
        unsafe { QueuedRwLockWriteGuard::new(self) }
    }
}

unsafe impl<'a, T: Send> Send for QueuedRwLockTicketGuard<'a, T> {}
unsafe impl<'a, T: Sync> Sync for QueuedRwLockTicketGuard<'a, T> {}

impl<'a, T> Drop for QueuedRwLockTicketGuard<'a, T> {
    fn drop(&mut self) {
        // This will only be called if we didn't take the lock, have to do so
        // or we stall other users forever
        self.lock.inner.write(self.ticket);
        self.lock.inner.write_unlock();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::TryLockError;
    use super::*;

    #[test]
    fn smoke() {
        let lock = QueuedRwLock::new(());
        drop(lock.read().unwrap());
        drop(lock.write().unwrap());
        drop((lock.read().unwrap(), lock.read().unwrap()));
        drop(lock.write().unwrap());
    }

    #[test]
    fn try_read() {
        let lock = QueuedRwLock::new(0);
        let write_guard = lock.write().unwrap();

        let read_result = lock.try_read();
        match read_result {
            Err(TryLockError::WouldBlock) => (),
            Ok(_) => assert!(false, "try_read should not succeed while write_guard is in scope"),
            Err(_) => assert!(false, "unexpected error"),
        }

        drop(write_guard);
    }

    #[test]
    fn try_write() {
        let lock = QueuedRwLock::new(0);
        let read_guard = lock.read().unwrap();

        let write_result = lock.try_write();
        match write_result {
            Err(TryLockError::WouldBlock) => (),
            Ok(_) => assert!(false, "try_write should not succeed while read_guard is in scope"),
            Err(_) => assert!(false, "unexpected error"),
        }

        drop(read_guard);
    }

    #[test]
    fn into_inner() {
        #[derive(Eq, PartialEq, Debug)]
        struct NonCopy(i32);

        let lock = QueuedRwLock::new(NonCopy(10));
        assert_eq!(lock.into_inner().unwrap(), NonCopy(10));
    }
}
