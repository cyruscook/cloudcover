use std::{ffi::CStr, os::raw::c_char};

unsafe extern "C" {
    fn CloudCoverAnalyzeGo(path: *const c_char) -> *mut c_char;
    fn CloudCoverFreeCString(ptr: *mut c_char);
    fn CloudCoverAnalyzeTerraform(path: *const c_char) -> *mut c_char;
    fn CloudCoverFreeTerraformCString(ptr: *mut c_char);
}

pub(crate) struct AnalyzerResponseGuard(*mut c_char);

impl AnalyzerResponseGuard {
    pub(crate) fn from_raw(ptr: *mut c_char) -> Self {
        Self(ptr)
    }

    pub(crate) fn as_c_str(&self) -> &CStr {
        unsafe { CStr::from_ptr(self.0) }
    }
}

impl Drop for AnalyzerResponseGuard {
    fn drop(&mut self) {
        unsafe { CloudCoverFreeCString(self.0) };
    }
}

pub(crate) fn analyze_go(path: *const c_char) -> *mut c_char {
    unsafe { CloudCoverAnalyzeGo(path) }
}

pub(crate) struct TerraformAnalyzerResponseGuard(*mut c_char);

impl TerraformAnalyzerResponseGuard {
    pub(crate) fn from_raw(ptr: *mut c_char) -> Self {
        Self(ptr)
    }
    pub(crate) fn as_c_str(&self) -> &CStr {
        unsafe { CStr::from_ptr(self.0) }
    }
}

impl Drop for TerraformAnalyzerResponseGuard {
    fn drop(&mut self) {
        unsafe { CloudCoverFreeTerraformCString(self.0) };
    }
}

pub(crate) fn analyze_terraform(path: *const c_char) -> *mut c_char {
    unsafe { CloudCoverAnalyzeTerraform(path) }
}
