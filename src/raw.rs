// Copyright (c) 2016 Travis Watkins
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::sync::{Condvar, Mutex};

pub struct RawQueuedRwLock {
    state: Mutex<State>,
    reader: Condvar,
    writer: Condvar,
}

impl RawQueuedRwLock {
    pub fn new() -> RawQueuedRwLock {
        RawQueuedRwLock {
            state: Mutex::new(State::new()),
            reader: Condvar::new(),
            writer: Condvar::new(),
        }
    }

    pub fn read(&self) {
        let mut state = self.state.lock().unwrap();

        while state.has_writer() {
            state = self.writer.wait(state).unwrap();
        }

        state.add_reader();
    }

    pub fn try_read(&self) -> bool {
        let mut state = self.state.lock().unwrap();

        if !state.has_writer() {
            state.add_reader();
            true
        } else {
            false
        }
    }

    pub fn read_unlock(&self) {
        let mut state = self.state.lock().unwrap();
        state.remove_reader();

        if state.has_writer() {
            if !state.has_readers() {
                self.reader.notify_all();
            }
        }
    }

    // Calls to take_ticket MUST eventually call write or else they will
    // deadlock all future callers
    pub fn take_ticket(&self) -> usize {
        let mut state = self.state.lock().unwrap();
        state.take_ticket()
    }

    pub fn write(&self, ticket: usize) {
        let mut state = self.state.lock().unwrap();

        while state.has_writer() && !state.is_next(ticket) {
            state = self.writer.wait(state).unwrap();
        }

        state.add_writer();

        while state.has_readers() {
            state = self.reader.wait(state).unwrap();
        }
    }

    // Only succeeds if there are no pending writes
    pub fn try_write_skip_queue(&self) -> bool {
        let mut state = self.state.lock().unwrap();

        if !state.has_writer() && !state.has_readers() && state.queue_empty() {
            state.take_ticket();
            state.add_writer();
            true
        } else {
            false
        }
    }

    pub fn write_unlock(&self) {
        let mut state = self.state.lock().unwrap();
        state.remove_writer();
        self.writer.notify_all();
    }
}

struct State {
    writer: bool,
    readers: usize,
    next_ticket: usize,
    total_tickets: usize,
}

impl State {
    fn new() -> State {
        State {
            writer: false,
            readers: 0,
            next_ticket: 0,
            total_tickets: 0,
        }
    }

    fn add_reader(&mut self) {
        self.readers += 1;
    }

    fn remove_reader(&mut self) {
        self.readers -= 1;
    }

    fn has_readers(&self) -> bool {
        self.readers != 0
    }

    fn add_writer(&mut self) {
        self.next_ticket += 1;
        self.writer = true;
    }

    fn remove_writer(&mut self) {
        self.writer = false;
    }

    fn has_writer(&self) -> bool {
        self.writer
    }

    fn take_ticket(&mut self) -> usize {
        let ticket = self.total_tickets;
        self.total_tickets += 1;
        ticket
    }

    fn is_next(&self, ticket: usize) -> bool {
        self.next_ticket == ticket
    }

    fn queue_empty(&self) -> bool {
        self.next_ticket == self.total_tickets - 1
    }
}
