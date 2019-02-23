# onecopy-rs
Provides a handle for a `Box<T>` that is `Clone+Send+Sync`, allowing at most 1 thread to consume the inner value.

This allows you to move things between threads that aren't normally allowed, by ensuring that only 1 thread has access to the internal value via an atomically swapped pointer.

In addition to the base `OneCopy` wrapper that allows the inner value to only be consumed once, `OneCopyCanReplace` allows the inner value to be replaced with another, like a (probably) safe multithreaded version of `std::cell::Cell`.

**Do NOT** use a real `std::cell::Cell` though, especially not behind `Rc`; this library uses `unsafe` and allows such things to compile, but threads can definitely race and cause memory corruption. It should be safe to move a `Rc` for which there are no other references however. See the `rc_share` test for an example of code that can produce a crash and should be avoided.
