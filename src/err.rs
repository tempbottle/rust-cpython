// Copyright (c) 2015 Daniel Grunwald
//
// Permission is hereby granted, free of charge, to any person obtaining a copy of this
// software and associated documentation files (the "Software"), to deal in the Software
// without restriction, including without limitation the rights to use, copy, modify, merge,
// publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons
// to whom the Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all copies or
// substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
// INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
// PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE
// FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
// OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

use std;
use python::{PythonObject, Python, ToPythonPointer, PythonObjectDowncastError, PythonObjectWithTypeObject};
use objects::{PyObject, PyType, exc};
#[cfg(feature="python27-sys")]
use objects::oldstyle::PyClass;
use ffi;
use libc;
use conversion::ToPyObject;
use std::ffi::CString;

/// Represents a Python exception that was raised.
#[derive(Clone, Debug)]
pub struct PyErr<'p> {
    /// The type of the exception. This should be either a `PyClass` or a `PyType`.
    pub ptype : PyObject<'p>,
    /// The value of the exception.
    /// 
    /// This can be either an instance of `ptype`,
    /// a tuple of arguments to be passed to `ptype`'s constructor,
    /// or a single argument to be passed to `ptype`'s constructor.
    /// Call `PyErr::instance()` to get the exception instance in all cases.
    pub pvalue : Option<PyObject<'p>>,
    /// The `PyTraceBack` object associated with the error.
    pub ptraceback : Option<PyObject<'p>>
}


/// Represents the result of a Python call.
pub type PyResult<'p, T> = Result<T, PyErr<'p>>;

impl <'p> PyErr<'p> {
    /// Gets whether an error is present in the Python interpreter's global state.
    #[inline]
    pub fn occurred(_ : Python<'p>) -> bool {
        unsafe { !ffi::PyErr_Occurred().is_null() }
    }

    /// Retrieves the current error from the Python interpreter's global state.
    /// The error is cleared from the Python interpreter.
    /// If no error is set, returns a `SystemError`.
    pub fn fetch(py : Python<'p>) -> PyErr<'p> {
        unsafe {
            let mut ptype      : *mut ffi::PyObject = std::mem::uninitialized();
            let mut pvalue     : *mut ffi::PyObject = std::mem::uninitialized();
            let mut ptraceback : *mut ffi::PyObject = std::mem::uninitialized();
            ffi::PyErr_Fetch(&mut ptype, &mut pvalue, &mut ptraceback);
            PyErr::new_from_ffi_tuple(py, ptype, pvalue, ptraceback)
        }
    }

    unsafe fn new_from_ffi_tuple(py: Python<'p>, ptype: *mut ffi::PyObject, pvalue: *mut ffi::PyObject, ptraceback: *mut ffi::PyObject) -> PyErr<'p> {
        // Note: must not panic to ensure all owned pointers get acquired correctly,
        // and because we mustn't panic in normalize().
        PyErr {
            ptype: if ptype.is_null() {
                        py.get_type::<exc::SystemError>().into_object()
                   } else {
                        PyObject::from_owned_ptr(py, ptype)
                   },
            pvalue: PyObject::from_owned_ptr_opt(py, pvalue),
            ptraceback: PyObject::from_owned_ptr_opt(py, ptraceback)
        }
    }

    /// Creates a new PyErr of type `T`.
    ///
    /// `value` can be:
    /// * `NoArgs`: the exception instance will be created using python `T()`
    /// * a tuple: the exception instance will be created using python `T(*tuple)`
    /// * any other value: the exception instance will be created using python `T(value)`
    ///
    /// Panics if `T` is not a python class derived from `BaseException`.
    pub fn new<T, V>(py: Python<'p>, value: V) -> PyErr<'p>
        where T: PythonObjectWithTypeObject<'p>, V: ToPyObject<'p>
    {
        PyErr::new_helper(py.get_type::<T>(), value.to_py_object(py).into_object())
    }

    fn new_helper(ty: PyType<'p>, value: PyObject<'p>) -> PyErr<'p> {
        assert!(unsafe { ffi::PyExceptionClass_Check(ty.as_object().as_ptr()) } != 0);
        PyErr {
            ptype: ty.into_object(),
            pvalue: Some(value),
            ptraceback: None
        }
    }

    /// Creates a new PyErr.
    ///
    /// `obj` must be an Python exception instance, the PyErr will use that instance.
    /// If `obj` is a Python exception type object, the PyErr will (lazily) create a new instance of that type.
    /// Otherwise, a `TypeError` is created instead.
    pub fn from_instance<O>(obj: O) -> PyErr<'p> where O: PythonObject<'p> {
        PyErr::from_instance_helper(obj.into_object())
    }

    fn from_instance_helper(obj: PyObject<'p>) -> PyErr<'p> {
        let py = obj.python();
        if unsafe { ffi::PyExceptionInstance_Check(obj.as_ptr()) } != 0 {
            PyErr {
                ptype: unsafe { PyObject::from_borrowed_ptr(py, ffi::PyExceptionInstance_Class(obj.as_ptr())) },
                pvalue: Some(obj),
                ptraceback: None
            }
        } else if unsafe { ffi::PyExceptionClass_Check(obj.as_ptr()) } != 0 {
            PyErr {
                ptype: obj,
                pvalue: None,
                ptraceback: None
            }
        } else {
            PyErr {
                ptype: py.get_type::<exc::TypeError>().into_object(),
                pvalue: Some("exceptions must derive from BaseException".to_py_object(py).into_object()),
                ptraceback: None
            }
        }
    }

    /// Construct a new error, with the usual lazy initialization of Python exceptions.
    /// `exc` is the exception type; usually one of the standard exceptions like `PyExc::runtime_error()`.
    /// `value` is the exception instance, or a tuple of arguments to pass to the exception constructor.
    #[inline]
    pub fn new_lazy_init(exc: PyType<'p>, value: Option<PyObject<'p>>) -> PyErr<'p> {
        PyErr {
            ptype: exc.into_object(),
            pvalue: value,
            ptraceback: None
        }
    }

    /// Print a standard traceback to sys.stderr.
    pub fn print(self) {
        self.restore();
        unsafe { ffi::PyErr_PrintEx(0) }
    }

    /// Print a standard traceback to sys.stderr.
    pub fn print_and_set_sys_last_vars(self) {
        self.restore();
        unsafe { ffi::PyErr_PrintEx(1) }
    }

    /// Return true if the current exception matches the exception in `exc`.
    /// If `exc` is a class object, this also returns `true` when `self` is an instance of a subclass.
    /// If `exc` is a tuple, all exceptions in the tuple (and recursively in subtuples) are searched for a match.
    #[inline]
    pub fn matches(&self, exc: &PyObject) -> bool {
        unsafe { ffi::PyErr_GivenExceptionMatches(self.ptype.as_ptr(), exc.as_ptr()) != 0 }
    }

    /// Normalizes the error. This ensures that the exception value is an instance of the exception type.
    pub fn normalize(&mut self) {
        // The normalization helper function involves temporarily moving out of the &mut self,
        // which requires some unsafe trickery:
        unsafe {
            std::ptr::write(self, std::ptr::read(self).into_normalized());
        }
        // This is safe as long as normalized() doesn't unwind due to a panic.
    }
    
    /// Helper function for normalizing the error by deconstructing and reconstructing the PyErr.
    /// Must not panic for safety in normalize()
    fn into_normalized(self) -> PyErr<'p> {
        let PyErr { ptype, pvalue, ptraceback } = self;
        let py = ptype.python();
        let mut ptype = ptype.steal_ptr();
        let mut pvalue = pvalue.steal_ptr();
        let mut ptraceback = ptraceback.steal_ptr();
        unsafe {
            ffi::PyErr_NormalizeException(&mut ptype, &mut pvalue, &mut ptraceback);
            PyErr::new_from_ffi_tuple(py, ptype, pvalue, ptraceback)
        }
    }

    /// Retrieves the exception type.
    ///
    /// If the exception type is an old-style class, returns `oldstyle::PyClass`.
    #[cfg(feature="python27-sys")]
    pub fn get_type(&self) -> PyType<'p> {
        let py = self.ptype.python();
        match self.ptype.clone().cast_into::<PyType>() {
            Ok(t)  => t,
            Err(_) =>
                match self.ptype.cast_as::<PyClass>() {
                    Ok(_)  => py.get_type::<PyClass>(),
                    Err(_) => py.None().get_type().clone()
                }
        }
    }

    /// Retrieves the exception type.
    #[cfg(not(feature="python27-sys"))]
    pub fn get_type(&self) -> PyType<'p> {
        let py = self.ptype.python();
        match self.ptype.clone().cast_into::<PyType>() {
            Ok(t)  => t,
            Err(_) => py.None().get_type().clone()
        }
    }

    /// Retrieves the exception instance for this error.
    /// This method takes `&mut self` because the error might need
    /// to be normalized in order to create the exception instance.
    pub fn instance(&mut self) -> PyObject<'p> {
        self.normalize();
        match self.pvalue {
            Some(ref instance) => instance.clone(),
            None => self.ptype.python().None()
        }
    }

    /// Writes the error back to the Python interpreter's global state.
    /// This is the opposite of `PyErr::fetch()`.
    #[inline]
    pub fn restore(self) {
        let PyErr { ptype, pvalue, ptraceback } = self;
        unsafe {
            ffi::PyErr_Restore(ptype.steal_ptr(), pvalue.steal_ptr(), ptraceback.steal_ptr())
        }
    }

    /// Issue a warning message.
    /// May return a PyErr if warnings-as-errors is enabled.
    pub fn warn(py: Python<'p>, category: &PyObject, message: &str, stacklevel: i32) -> PyResult<'p, ()> {
        let message = CString::new(message).unwrap();
        unsafe {
            error_on_minusone(py, ffi::PyErr_WarnEx(category.as_ptr(), message.as_ptr(), stacklevel as ffi::Py_ssize_t))
        }
    }
}

/// Converts `PythonObjectDowncastError` to Python `TypeError`.
impl <'p> std::convert::From<PythonObjectDowncastError<'p>> for PyErr<'p> {
    fn from(err: PythonObjectDowncastError<'p>) -> PyErr<'p> {
        PyErr::new_lazy_init(err.0.get_type::<exc::TypeError>(), None)
    }
}

/// Construct PyObject from the result of a Python FFI call that returns a new reference (owned pointer).
/// Returns `Err(PyErr)` if the pointer is `null`.
/// Unsafe because the pointer might be invalid.
#[inline]
pub unsafe fn result_from_owned_ptr(py : Python, p : *mut ffi::PyObject) -> PyResult<PyObject> {
    if p.is_null() {
        Err(PyErr::fetch(py))
    } else {
        Ok(PyObject::from_owned_ptr(py, p))
    }
}

fn panic_after_error(_py: Python) -> ! {
    unsafe { ffi::PyErr_Print(); }
    panic!("Python API called failed");
}

#[inline]
pub unsafe fn from_owned_ptr_or_panic(py : Python, p : *mut ffi::PyObject) -> PyObject {
    if p.is_null() {
        panic_after_error(py);
    } else {
        PyObject::from_owned_ptr(py, p)
    }
}

pub unsafe fn result_cast_from_owned_ptr<'p, T>(py : Python<'p>, p : *mut ffi::PyObject) -> PyResult<'p, T>
    where T: ::python::PythonObjectWithCheckedDowncast<'p>
{
    if p.is_null() {
        Err(PyErr::fetch(py))
    } else {
        Ok(try!(PyObject::from_owned_ptr(py, p).cast_into()))
    }
}

pub unsafe fn cast_from_owned_ptr_or_panic<'p, T>(py : Python<'p>, p : *mut ffi::PyObject) -> T
    where T: ::python::PythonObjectWithCheckedDowncast<'p>
{
    if p.is_null() {
        panic_after_error(py);
    } else {
        PyObject::from_owned_ptr(py, p).cast_into().unwrap()
    }
}

/// Returns Ok if the error code is not -1.
#[inline]
pub fn error_on_minusone(py : Python, result : libc::c_int) -> PyResult<()> {
    if result != -1 {
        Ok(())
    } else {
        Err(PyErr::fetch(py))
    }
}

#[cfg(test)]
mod tests {
    use {Python, PyErr};
    use objects::exc;

    #[test]
    fn set_typeerror() {
        let gil = Python::acquire_gil();
        let py = gil.python();
        PyErr::new_lazy_init(py.get_type::<exc::TypeError>(), None).restore();
        assert!(PyErr::occurred(py));
        drop(PyErr::fetch(py));
    }
}


