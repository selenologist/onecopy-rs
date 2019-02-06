#![feature(test)]

use std::sync::{Arc, atomic::{AtomicPtr, Ordering}};

/// OneCopy is a thread-shareable handle that can have the inner value consumed by a single thread
/// only once, after which it is inaccessible.
///
/// Possible uses include a job queue in which the task is broadcast to several threads, but only 1
/// actually handles it. E.g. this avoids needing to select a thread, instead the first one to
/// retrieve the value gets it.
#[derive(Clone, Debug)]
pub struct OneCopy<T>(Arc<OneCopyInner<T>>);

impl<T> OneCopy<T> {
    //! takes a Box so that it can be converted to a raw pointer
    //! note that the destructor will not be called unless the value is converted back into a
    //! non-raw-pointer type.
    
    pub fn new<B>(v: B) -> Self
        where B: Into<Box<T>>
    {
        Self(Arc::new(OneCopyInner(AtomicPtr::new(Box::into_raw(v.into())))))
    }

    /// None if already taken
    pub fn get(&self) -> Option<Box<T>> {
        self.0.get()
    }
}

/// This is a version of OneCopy that can replace an empty value with a new one.
#[derive(Clone, Debug)]
pub struct OneCopyCanReplace<T>(Arc<OneCopyInner<T>>);

impl<T> OneCopyCanReplace<T> {
    // same as for OneCopy:

    pub fn new<B>(v: B) -> Self
        where B: Into<Box<T>>
    {
        Self(Arc::new(OneCopyInner(AtomicPtr::new(Box::into_raw(v.into())))))
    } 

    /// None if already taken
    pub fn get(&self) -> Option<Box<T>> {
        self.0.get()
    }

    // not found in OneCopy:

    pub fn empty() -> Self
    {
        Self(Arc::new(OneCopyInner(AtomicPtr::new(std::ptr::null_mut()))))
    }

    /// replaces an empty pointer, returning the argument the pointer was valid (e.g. not empty)
    /// e.g. returns None if the value was replaced, otherwise returns Some<T> with the argument.
    /// Valid pointers are not replaced.
    pub fn replace<B>(&self, v: B) -> Option<Box<T>>
        where B: Into<Box<T>>
    {
        self.0.replace(v.into())
    }

    /// replaces the existing pointer, returning the existing value if the existing pointer was
    /// valid.
    /// e.g. returns Some<T> with the old value if it was valid, and None otherwise.
    /// Always replaces the pointer.
    pub fn swap<B>(&self, v: B) -> Option<Box<T>>
        where B: Into<Box<T>>
    {
        self.0.swap(v.into())
    }
}

/// Because OneCopy wraps an Arc, there must be a type inside the Arc to catch the Drop of the Arc,
/// when there are no references left, to ensure that the allocated value is freed.
/// Had we only captured Drop in OneCopy itself, we would get a Drop for every Clone, which would mean
/// the value disappeared any time a OneCopy was dropped. This way we only get a Drop when there are
/// NO remaining references to the value the OneCopy protects.
///
/// Do not attempt to produce or obtain this value by any means, accessing it is unsafe.
#[derive(Debug)]
struct OneCopyInner<T>(AtomicPtr<T>);

impl<T> OneCopyInner<T> {
    fn get(&self) -> Option<Box<T>> {
        // take singular ownership of the pointer by swapping a null pointer in its place
        let null_ptr = std::ptr::null_mut();
        self.swap_ptr(null_ptr)
    }

    // the following functions are only called by Self
    
    // swap ptr for old value, returning old value as Some<Box<T>> if possible
    fn swap_ptr(&self, new_ptr: *mut T) -> Option<Box<T>> {
        let old_ptr = self.0.swap(new_ptr, Ordering::SeqCst);
        
        if old_ptr.is_null() {
            // value already taken
            None
        }
        else {
            unsafe {
                Some(Box::from_raw(old_ptr))
            }
        }
    }

    // the following functions are only called by OneCopyCanReplace
    
    fn replace(&self, v: Box<T>) -> Option<Box<T>> {
        let v_ptr = Box::into_raw(v);
        
        // replace old pointer with v_ptr if and only if old pointer is null
        let old = self.0.compare_and_swap(
            std::ptr::null_mut(), // old must equal
            v_ptr,
            Ordering::SeqCst
        );

        if old.is_null() {
            // was null, therefore the new value has replaced the old
            None
        }
        else {
            // old pointer was valid, therefore no swap happened.
            // Obtain v as a box again (do NOT use the old pointer)
            unsafe {
                Some(Box::from_raw(v_ptr))
            }
        }
    }

    fn swap(&self, v: Box<T>) -> Option<Box<T>> {
        self.swap_ptr(Box::into_raw(v))
    }

}

impl<T> std::ops::Drop for OneCopyInner<T> { 
    fn drop(&mut self) {
        // Because Box::into_raw calls mem::forget, we must ensure that the value is freed if
        // OneCopy::get() is never called. The easiest way is to simply call .get(), which will
        // convert the raw pointer back into a Box which will have its destructor called.
        let _ = self.get();
    }
}

// Tests

#[cfg(test)] extern crate test;
#[cfg(test)]
mod tests {
    use super::{OneCopy, OneCopyCanReplace};
    use super::test::Bencher;
    
    use std::thread;
    use std::sync::{Arc, Barrier, mpsc::channel};

    const N_THREADS: usize = 8; // number of threads to use for threaded tests

    const V_CONST: u32 = 420; // just some constant, not important

    #[test]
    fn single_thread() {
        let o = OneCopy::new(V_CONST);

        let first  = o.get();
        let second = o.get();
        
        let first_valid =
            match first {
                Some(b) => *b == V_CONST, // first access must match value
                None    => false
            };
        let second_valid =
            match second {
                Some(_) => false, // second access should fail
                None    => true
            };

        assert!(first_valid);
        assert!(second_valid);
    }

    #[test]
    fn drop_cloned() {
        let o = OneCopy::new(V_CONST);

        let copy = o.clone();
        std::mem::drop(o); // this drop should not free the actual memory

        let value = copy.get();
        
        let valid =
            match value {
                Some(b) => *b == V_CONST, // must match value
                None    => false
            };

        assert!(valid);
    }
    
    #[test]
    fn debug() { // tests if debug printing causes panic
        let o = OneCopy::new(V_CONST);
        println!("first {:?}", o);
        println!("value {:?}", o.get());
        println!("second {:?}", o);
        println!("value {:?}", o.get());
    }

    fn multiple_thread_main() {
        let o = OneCopy::new(V_CONST);

        // for kicking off all threads at once after initialisation
        // not strictly necessary but makes all the reads try to happen at once
        let barrier = Arc::new(Barrier::new(N_THREADS + 1));

        // for retrieving results
        let (tx, rx) = channel();
        for i in 0..N_THREADS {
            let tx = tx.clone();
            let o  =  o.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                tx.send((i, o.get())).unwrap()
            });
        }

        // manually drop our own tx
        std::mem::drop(tx);
        
        // start threads by having the master thread wait on the barrier too
        barrier.wait();

        // how many threads managed to get the value (should be 1)
        let mut n_got = 0;
        rx.iter()
          .for_each(|(i, b)|
               match b {
                   Some(b) => {
                       println!("thread {} got the value", i);
                       assert_eq!(*b, V_CONST, "got the OneCopy's inner value but it was incorrect");
                       n_got += 1;
                   }
                   None => {
                       //println!("thread {} didn't get the value", i);
                   }
               }
          );
        assert_eq!(n_got, 1, "more than 1 thread managed to get the OneCopy's inner value");
    }

    #[test]
    fn multiple_thread_once(){
        multiple_thread_main();
    }
 
    #[test]
    fn single_thread_replace() {
        let o = OneCopyCanReplace::empty();

        let try_replace =
            || match o.replace(V_CONST) {
                Some(_) => false, // did not replace, was full
                None    => true   // did replace
            };

        let try_get =
            || match o.get() {
                Some(b) => *b == V_CONST, // first access must match value
                None    => false
            };

        assert_eq!(try_get()    , false, "first get should fail (empty)");
        assert_eq!(try_replace(), true , "first replace should succeed (empty)");
        assert_eq!(try_replace(), false, "second replace should fail (empty)");
        assert_eq!(try_get()    , true , "second get should succeed (empty)");
    }

    #[test]
    fn multiple_thread_replace() {
        let o = OneCopyCanReplace::empty();

        // for retrieving results
        let (tx, rx) = channel();

        // reader and writer threads do inefficient spinloops until their operations succeed. Don't
        // actually do this in practice.

        // spawn reader threads (must equal number of writers)
        for reader in 0..N_THREADS {
            let tx = tx.clone();
            let o  =  o.clone();
            thread::spawn(move || {
                // spinloops until received 3 messages, forwarding them to main result channel
                for _ in 0..3 {
                    loop {
                        if let Some(boxed) = o.get() {
                            tx.send((reader, Some(boxed))).unwrap();
                            break;
                        }
                    }
                }
                // send message that reader has finished its three messages
                tx.send((reader, None)).unwrap();
            });
        }

        // manually drop our own tx
        std::mem::drop(tx);

        // spawn writer threads
        for writer in 0..N_THREADS {
            let o  = o.clone();
            thread::spawn(move || {
                // spinloops until managed to send 3 messages
                for n in 0..3 {
                    let mut n_box = Box::new((writer, n));
                    while let Some(didnt_replace) = o.replace(n_box) {
                        n_box = didnt_replace;
                    }
                }
            });
        }

        // get results
        let mut n_got = 0;
        rx.iter()
          .for_each(|(reader, opt_msg)| match opt_msg {
            Some(boxed) => {
                n_got += 1;
                let (writer, msg) = *boxed;
                println!("reader {} got from writer {} step {}", reader, writer, msg)
            },
            None =>
                println!("reader {} finished", reader)
          });

        assert_eq!(n_got, N_THREADS * 3, "A writer should have produced 3 messages per thread");
    }


    #[bench]
    fn multiple_thread_bench(b: &mut Bencher){
        // n.b. does the thread creation every iteration too; not a good representation of
        // performance but good to do a quick maybe-false-negative check for races
        b.iter(multiple_thread_main);
    }
}
