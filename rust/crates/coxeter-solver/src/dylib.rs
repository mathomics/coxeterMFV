//! Opening a solver's C library at run time.
//!
//! [`c_api!`] declares a table of a library's functions, typed as the
//! vendor's header declares them, and looks each one up by name.
//! [`open_library`] opens the library from its full path.

use std::path::Path;

/// Declares `struct $api` holding one function pointer per C function, and
/// `$api::load(library)`, which looks every one up by name and fails naming
/// the first one missing. The library is kept in the table, so it stays open
/// while any pointer can be called.
///
/// The types are the C prototypes of the vendor's header; `"system"` is the
/// `__stdcall` those headers ask for (the same as `"C"` on 64-bit targets).
macro_rules! c_api {
    ($vis:vis struct $api:ident { $($name:ident: fn($($arg:ty),* $(,)?) $(-> $ret:ty)?;)* }) => {
        #[allow(non_snake_case)]
        $vis struct $api {
            $(pub(crate) $name: unsafe extern "system" fn($($arg),*) $(-> $ret)?,)*
            _library: ::libloading::Library,
        }

        impl $api {
            // The locals carry the C names, as the fields do.
            #[allow(non_snake_case)]
            pub(crate) fn load(library: ::libloading::Library) -> ::std::result::Result<$api, String> {
                $(
                    // SAFETY: the pointer type is the function's prototype in
                    // the vendor's C header, and the library outlives it.
                    let $name = match unsafe {
                        library.get::<unsafe extern "system" fn($($arg),*) $(-> $ret)?>(concat!(stringify!($name), "\0").as_bytes())
                    } {
                        Ok(symbol) => *symbol,
                        Err(_) => return Err(format!("the library has no function {}", stringify!($name))),
                    };
                )*
                Ok($api { $($name,)* _library: library })
            }
        }
    };
}

pub(crate) use c_api;

/// Open a shared library from its full path.
///
/// On Windows the library's own folder is searched for the libraries it
/// needs in turn (`LOAD_WITH_ALTERED_SEARCH_PATH`), as an installed solver
/// expects; elsewhere the platform's loader rules apply.
pub(crate) fn open_library(path: &Path) -> Result<libloading::Library, String> {
    let path = std::path::absolute(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // SAFETY: loading runs the library's initialisers. The libraries opened
    // here are the solvers' own, found where their installers put them or
    // chosen by the user.
    #[cfg(windows)]
    let library = unsafe {
        libloading::os::windows::Library::load_with_flags(&path, libloading::os::windows::LOAD_WITH_ALTERED_SEARCH_PATH)
    }
    .map(libloading::Library::from);
    #[cfg(not(windows))]
    let library = unsafe { libloading::Library::new(&path) };
    library.map_err(|e| format!("{}: {e}", path.display()))
}
