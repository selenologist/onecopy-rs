# onecopy-rs
Provides a handle for a `Box<T>` that is `Clone+Send+Sync`, allowing at most 1 thread to consume the inner value.

This allows you to move things between threads that aren't normally allowed, by ensuring that only 1 thread has access to the internal value via an atomically swapped pointer.

In addition to the base `OneCopy` wrapper that allows the inner value to only be consumed once, `OneCopyCanReplace` allows the inner value to be replaced with another, like a multithreaded version of `std::cell::Cell`.

This library uses `unsafe` and may possibly break things.
