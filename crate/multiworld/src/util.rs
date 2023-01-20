use pyo3::{
    once_cell::GILOnceCell,
    prelude::*,
};

pub(crate) struct PyLazy<T> {
    cell: GILOnceCell<T>,
    init: for<'r> fn(Python<'r>) -> T,
}

impl<T> PyLazy<T> {
    pub(crate) const fn new(init: for<'r> fn(Python<'r>) -> T) -> Self {
        Self {
            cell: GILOnceCell::new(),
            init,
        }
    }

    pub(crate) fn get(&self, py: Python<'_>) -> &T {
        self.cell.get_or_init(py, || (self.init)(py))
    }
}
