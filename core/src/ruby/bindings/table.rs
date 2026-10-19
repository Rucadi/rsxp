// SPDX-License-Identifier: GPL-2.0-or-later
//! Ruby-GC-owned RGSS `Table` objects, bound directly through Magnus.
//!
//! `RubyTable` wraps `RefCell<Option<Table>>` (the engine `crate::table::Table`)
//! plus a `Cell<u64>` revision counter, and is registered as the `Table` class.
//! The game-visible API is layered on top of the private `__` methods by
//! `rgss.rb`: `__initialize`, `__get`/`__set`, `__resize`, `__dimensions` and
//! `__rsxp_revision`. The revision is bumped on every value-changing mutation and
//! lets the retained map and tilemap caches detect edits without re-reading every
//! cell.
//!
//! Tables are capped at 16,777,216 elements (`ArgumentError` beyond that); a
//! write only bumps the revision when the stored `i16` actually changes.
//! `_dump`/`_load` provide Marshal serialization, and `initialize_copy` deep
//! copies the table and bumps the revision.
use magnus::{function, method, prelude::*, Error, RString, Ruby};
use crate::table::Table;
use std::cell::{Cell, RefCell};

#[derive(Default)]
#[magnus::wrap(class = "Table", free_immediately, size)]
pub(super) struct RubyTable(pub(super) RefCell<Option<Table>>, Cell<u64>);

impl RubyTable {
    fn initialize(ruby: &Ruby, this: &Self, x: usize, y: usize, z: usize) -> Result<(), Error> {
        Self::check_size(ruby, x, y, z)?;
        let table = Table::new(x, y, z)
            .map_err(|e| Error::new(ruby.exception_arg_error(), e.to_string()))?;
        *this.0.borrow_mut() = Some(table);
        this.1.set(this.1.get().wrapping_add(1));
        Ok(())
    }
    fn get(&self, x: i64, y: i64, z: i64) -> Option<i16> {
        self.0.borrow().as_ref()?.get(
            usize::try_from(x).ok()?,
            usize::try_from(y).ok()?,
            usize::try_from(z).ok()?,
        )
    }
    fn set(&self, x: i64, y: i64, z: i64, value: i64) -> i16 {
        if let (Ok(x), Ok(y), Ok(z), Some(table)) = (
            usize::try_from(x),
            usize::try_from(y),
            usize::try_from(z),
            self.0.borrow_mut().as_mut(),
        ) && table.get(x, y, z).is_some_and(|old| old != value as i16)
        {
            table.set(x, y, z, value as i16);
            self.1.set(self.1.get().wrapping_add(1));
        }
        value as i16
    }
    pub(super) fn revision(&self) -> u64 {
        self.1.get()
    }
    fn resize(ruby: &Ruby, this: &Self, x: usize, y: usize, z: usize) -> Result<(), Error> {
        Self::check_size(ruby, x, y, z)?;
        if this.0.borrow().is_none() {
            return Self::initialize(ruby, this, x, y, z);
        }
        if let Some(table) = this.0.borrow_mut().as_mut() {
            table
                .resize(x, y, z)
                .map_err(|e| Error::new(ruby.exception_arg_error(), e.to_string()))?;
            this.1.set(this.1.get().wrapping_add(1));
        }
        Ok(())
    }
    fn check_size(ruby: &Ruby, x: usize, y: usize, z: usize) -> Result<(), Error> {
        if x.checked_mul(y)
            .and_then(|n| n.checked_mul(z))
            .is_none_or(|n| n > 16_777_216)
        {
            return Err(Error::new(
                ruby.exception_arg_error(),
                "Table exceeds the 16 million element limit",
            ));
        }
        Ok(())
    }
    fn dimensions(&self) -> (usize, usize, usize) {
        let [x, y, z] = self
            .0
            .borrow()
            .as_ref()
            .map(Table::dimensions)
            .unwrap_or([0; 3]);
        (x, y, z)
    }
    fn dump(ruby: &Ruby, this: &Self, _depth: i64) -> RString {
        let bytes = this
            .0
            .borrow()
            .as_ref()
            .map(Table::serialize)
            .unwrap_or_else(|| Table::new(0, 1, 1).unwrap().serialize());
        ruby.str_from_slice(&bytes)
    }
    fn load(ruby: &Ruby, string: RString) -> Result<Self, Error> {
        // SAFETY: no Ruby calls occur until the borrowed bytes have been consumed.
        let result = Table::deserialize(unsafe { string.as_slice() });
        result
            .map(|table| Self(RefCell::new(Some(table)), Cell::new(0)))
            .map_err(|e| Error::new(ruby.exception_arg_error(), e.to_string()))
    }
    fn initialize_copy(&self, other: &Self) {
        let table = other.0.borrow().clone();
        *self.0.borrow_mut() = table;
        self.1.set(self.1.get().wrapping_add(1));
    }
}

pub(super) fn bind(ruby: &Ruby) -> Result<(), Error> {
    let class = ruby.define_class("Table", ruby.class_object())?;
    class.define_alloc_func::<RubyTable>();
    class.define_method("__initialize", method!(RubyTable::initialize, 3))?;
    class.define_method("__get", method!(RubyTable::get, 3))?;
    class.define_method("__set", method!(RubyTable::set, 4))?;
    class.define_method("__resize", method!(RubyTable::resize, 3))?;
    class.define_method("__dimensions", method!(RubyTable::dimensions, 0))?;
    class.define_method("__rsxp_revision", method!(RubyTable::revision, 0))?;
    class.define_method("_dump", method!(RubyTable::dump, 1))?;
    class.define_singleton_method("_load", function!(RubyTable::load, 1))?;
    class.define_method("initialize_copy", method!(RubyTable::initialize_copy, 1))?;
    Ok(())
}
