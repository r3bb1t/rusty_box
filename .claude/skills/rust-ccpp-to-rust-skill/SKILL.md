---
name: rust-ccpp-to-rust-conversion
description: Best practices for converting C/C++ code to idiomatic Rust with proper error handling, type-safe invariants, algebraic types, and eliminating undefined behavior. Use this skill when rewriting C/C++ code to Rust, converting panics to Result-based errors, modeling implicit invariants as type-system guarantees, and leveraging Rust's safety without sacrificing performance.
license: MIT
---

# Converting C/C++ Code to Idiomatic Rust

This skill guides conversion of C/C++ code to production-grade Rust that leverages the type system to eliminate entire classes of bugs while maintaining or improving performance.

## Core Conversion Principles

### 1. Replace Panics with Errors

**Rule**: Any code path that would call `panic!()`, `unwrap()`, `expect()`, or unsafe code that could fail must return a descriptive error wrapped in a `Result`.

**C/C++ to Rust panic replacements:**

```rust
// C: Array access without bounds check
// int arr[10]; int x = arr[index];  // UNDEFINED if index >= 10
// Rust equivalent:
fn get_element(slice: &[i32], index: usize) -> Result<i32, IndexError> {
    slice.get(index)
        .copied()
        .ok_or(IndexError { index, len: slice.len() })
}

// C: Pointer dereference without null check
// int* ptr = ...; int x = *ptr;  // UNDEFINED if ptr is NULL
// Rust equivalent:
fn dereference(ptr: Option<&i32>) -> Result<i32, NullPointerError> {
    ptr.copied().ok_or(NullPointerError)
}

// C: Integer overflow
// int x = INT_MAX; x++;  // UNDEFINED BEHAVIOR
// Rust equivalent:
fn add_safe(a: i32, b: i32) -> Result<i32, OverflowError> {
    a.checked_add(b).ok_or(OverflowError { a, b })
}

// C: String parsing without validation
// int x = atoi(str);  // Returns 0 on invalid input, silent failure
// Rust equivalent:
fn parse_int(s: &str) -> Result<i32, ParseError> {
    s.trim().parse::<i32>()
        .map_err(|_| ParseError::InvalidInteger(s.to_string()))
}
```

### 2. Convert Implicit Integer Enums to Rust Enums

**Rule**: When a C/C++ `int` parameter represents a fixed set of values, convert it to a Rust `enum`. This makes the API self-documenting and eliminates invalid state.

**Pattern: Finding Hidden Enums in C/C++**

```c
// C code: Flags encoded as int constants
#define MODE_READ 0
#define MODE_WRITE 1
#define MODE_APPEND 2

void open_file(const char* path, int mode) {
    if (mode == MODE_READ) { ... }
    else if (mode == MODE_WRITE) { ... }
    else if (mode == MODE_APPEND) { ... }
    // No validation - what if mode=99?
}
```

```rust
// Idiomatic Rust: Type-safe enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    Read,
    Write,
    Append,
}

pub fn open_file(path: &str, mode: FileMode) -> Result<File, IoError> {
    match mode {
        FileMode::Read => { /* ... */ }
        FileMode::Write => { /* ... */ }
        FileMode::Append => { /* ... */ }
    }
    // Compiler ensures all variants handled
}
```

**Pattern: Status Codes and Return Values**

```c
// C: Magic integer returns
#define SUCCESS 0
#define ERR_NOT_FOUND 1
#define ERR_PERMISSION 2
#define ERR_IO 3

int process_file(const char* path) {
    // Caller must know what codes mean
    return ERR_NOT_FOUND;
}
```

```rust
// Rust: Explicit error types
#[derive(Error, Debug)]
pub enum ProcessError {
    #[error("file not found: {path}")]
    NotFound { path: String },
    
    #[error("permission denied")]
    PermissionDenied,
    
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn process_file(path: &str) -> Result<Output, ProcessError> {
    // Errors are explicit and self-documenting
}
```

### 3. Replace Bit Magic with `bitflags` Crate

**Rule**: Replace C-style bit operations with the `bitflags` crate for clarity, type safety, and maintainability.

**C/C++ Bit Manipulation:**

```c
// C: Flags as bit constants
#define FLAG_READ    0x01
#define FLAG_WRITE   0x02
#define FLAG_EXECUTE 0x04
#define FLAG_DELETE  0x08

typedef unsigned int Permissions;

void check_perms(Permissions p) {
    if (p & FLAG_READ) { /* has read */ }
    if (p & FLAG_WRITE) { /* has write */ }
    
    // Easy to make mistakes with magic numbers
    p |= 0x10;  // What is 0x10? Unknown!
    p &= ~FLAG_EXECUTE;  // Could typo and lose safety
}
```

```rust
// Idiomatic Rust with bitflags crate
use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Permissions: u32 {
        const READ = 0x01;
        const WRITE = 0x02;
        const EXECUTE = 0x04;
        const DELETE = 0x08;
    }
}

pub fn check_perms(p: Permissions) {
    if p.contains(Permissions::READ) { /* has read */ }
    if p.contains(Permissions::WRITE) { /* has write */ }
    
    // Type-safe operations
    let mut perms = p;
    perms.insert(Permissions::READ);
    perms.remove(Permissions::EXECUTE);
    
    // Bitflags handles iteration too
    for flag in p.iter() {
        println!("Flag: {:?}", flag);
    }
}
```

### 4. Convert Pointers and Sizes to Slices

**Rule**: Replace C-style `pointer + size` pairs with Rust slices. Slices encode bounds information in the type system.

**C/C++ Pointer Pairs:**

```c
// C: Separate pointer and size - easy to get out of sync
void process_buffer(const uint8_t* data, size_t len) {
    for (size_t i = 0; i < len; i++) {
        uint8_t byte = data[i];  // Bounds check depends on caller
    }
    // What if 'data' points to 5 bytes but 'len' is 100? UB!
}

// Common mistake:
process_buffer(array_ptr, wrong_size);  // Silent corruption
```

```rust
// Idiomatic Rust: Slice encodes bounds
pub fn process_buffer(data: &[u8]) -> Result<(), ProcessError> {
    // Length is part of the slice type
    for (i, &byte) in data.iter().enumerate() {
        // Iteration is safe and bounds-checked
    }
    // Impossible to pass wrong length - it's in the slice reference
}

// Caller must pass a valid slice:
let data = &my_array[..];           // Slice of entire array
let data = &my_array[5..10];        // Subslice
let data = vec.as_slice();          // Vec to slice
// Compiler ensures these are valid
```

**Pattern: C Function Returning Data**

```c
// C: Caller must allocate buffer and pass size
// Caller is responsible for sizing
size_t read_data(uint8_t* output, size_t max_len) {
    // Return actual bytes written
    // Caller must trust that output is large enough
    memcpy(output, ..., actual_bytes);
    return actual_bytes;
}

// Usage:
uint8_t buffer[100];
size_t bytes = read_data(buffer, 100);
// What if actual_bytes > 100? Buffer overflow!
```

```rust
// Idiomatic Rust: Return owned data
pub fn read_data() -> Result<Vec<u8>, IoError> {
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut /* reader */, &mut buf)?;
    Ok(buf)  // Caller gets ownership, no guessing sizes
}

// Or preallocate for performance:
pub fn read_data_into(output: &mut [u8]) -> Result<usize, IoError> {
    // Function cannot write past output.len()
    // Bounds are encoded in the slice type
    let n = /* reader */.read(output)?;
    Ok(n)
}
```

### 5. Define Errors with `thiserror` Crate

**Rule**: Use `thiserror` for all error types. Make errors specific and include context without allocating unnecessarily.

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConversionError {
    #[error("invalid UTF-8 sequence")]
    InvalidUtf8(#[from] std::string::FromUtf8Error),
    
    #[error("index {index} out of bounds (max: {max})")]
    IndexOutOfBounds { index: usize, max: usize },
    
    #[error("null pointer dereference")]
    NullPointer,
    
    #[error("integer overflow: {op}")]
    IntegerOverflow { op: &'static str },
    
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("permission denied: {reason}")]
    PermissionDenied { reason: String },
}
```

### 6. Model Invariants as Type-System Guarantees

**Rule**: Use newtypes, phantom types, and builder patterns to make invalid states unrepresentable.

**Pattern: Validated Inputs**

```c
// C: Struct with implicit invariants
typedef struct {
    int age;        // Must be 0-150
    char* email;    // Must be valid email
    int status;     // Must be 0-3
} User;

// No enforcement - anyone can create invalid User
User u = {.age = 500, .email = NULL, .status = 99};
```

```rust
// Rust: Invariants encoded in types
#[derive(Clone, Debug)]
pub struct Age(u8);

impl Age {
    pub fn new(age: u8) -> Result<Self, ValidationError> {
        if age <= 150 {
            Ok(Age(age))
        } else {
            Err(ValidationError::AgeOutOfRange(age))
        }
    }
    pub fn as_u8(&self) -> u8 { self.0 }
}

#[derive(Clone, Debug)]
pub struct Email(String);

impl Email {
    pub fn new(email: String) -> Result<Self, ValidationError> {
        if is_valid_email(&email) {
            Ok(Email(email))
        } else {
            Err(ValidationError::InvalidEmail(email))
        }
    }
    pub fn as_str(&self) -> &str { &self.0 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserStatus {
    Active,
    Inactive,
    Suspended,
    Deleted,
}

pub struct User {
    age: Age,
    email: Email,
    status: UserStatus,
}

impl User {
    pub fn new(age: Age, email: Email, status: UserStatus) -> Self {
        User { age, email, status }
    }
}

// Now all fields are guaranteed valid - construction enforces invariants
```

**Pattern: State Machines with Types**

```c
// C: State machine with hidden states
typedef struct {
    int state;  // 0=closed, 1=opening, 2=open, 3=closing
    void* data;
} Connection;

void send_data(Connection* c, const char* data) {
    if (c->state == 2) {  // Magic number for "open"
        // Actually, if state is 1 (opening), this might race
    }
}
```

```rust
// Rust: Impossible to be in wrong state
pub enum Connection {
    Closed,
    Opening(OpeningState),
    Open(OpenConnection),
    Closing,
}

impl Connection {
    pub fn send_data(self, data: &[u8]) -> Result<Self, SendError> {
        match self {
            Connection::Open(conn) => {
                // Only this variant can send
                conn.send(data)?;
                Ok(Connection::Open(conn))
            }
            _ => Err(SendError::NotConnected),
        }
    }
}
```

### 7. Avoid Unnecessary Allocations

**Rule**: Use owned types (String, Vec) only when necessary. Prefer references, borrowed slices, and stack allocation.

**C/C++ String Handling:**

```c
// C: String copies everywhere
char* concat(const char* a, const char* b) {
    char* result = malloc(strlen(a) + strlen(b) + 1);
    strcpy(result, a);
    strcat(result, b);
    return result;  // Caller must free!
}

// User must remember to free
char* s = concat("hello", "world");
// ... use s ...
free(s);  // Easy to forget!
```

```rust
// Idiomatic Rust: Borrow where possible
pub fn concat(a: &str, b: &str) -> String {
    // Allocate once for exact size needed
    let mut result = String::with_capacity(a.len() + b.len());
    result.push_str(a);
    result.push_str(b);
    result
}

// Or for operations that don't need ownership:
pub fn print_concat(a: &str, b: &str) {
    // No allocation needed
    println!("{}{}", a, b);
}

// For multiple strings, collect without intermediate copies:
pub fn join_strings(strings: &[&str], sep: &str) -> String {
    strings.join(sep)  // Single allocation
}
```

**Pattern: Avoiding Vec Copies**

```c
// C: Function that modifies and returns vector
int* process_array(int* data, size_t len) {
    int* result = malloc(len * sizeof(int));
    for (size_t i = 0; i < len; i++) {
        result[i] = data[i] * 2;
    }
    return result;
}

// Must free original and result separately
```

```rust
// Rust: In-place modification when possible
pub fn double_elements(data: &mut [i32]) {
    for elem in data.iter_mut() {
        *elem = elem.saturating_mul(2);  // Safe multiplication
    }
}

// If allocation is needed, reuse input:
pub fn process_elements(mut data: Vec<i32>) -> Vec<i32> {
    for elem in data.iter_mut() {
        *elem = elem.saturating_mul(2);
    }
    data  // Move, not copy
}

// Or use iterators for functional style without allocation:
let processed: Vec<i32> = data
    .into_iter()
    .map(|x| x.saturating_mul(2))
    .collect();
```

### 8. Safety vs. Performance: Borrow Checker Solutions

**Rule**: Keep code safe by default. Only use `Cell`/`UnsafeCell` when the borrow checker prevents legitimate patterns and safety is maintained.

**Pattern: Shared Mutable State**

```c
// C: Shared state with manual locking (or no locking!)
typedef struct {
    int counter;
    void (*callback)(int);
} EventHandler;

void trigger(EventHandler* h) {
    h->counter++;
    h->callback(h->counter);  // Callback might try to modify EventHandler!
}
```

```rust
// Rust with Cell: Interior mutability without allocation
use std::cell::Cell;

pub struct EventHandler {
    counter: Cell<i32>,
    callback: Option<Box<dyn Fn(i32)>>,
}

impl EventHandler {
    pub fn trigger(&self) {
        let new_count = self.counter.get() + 1;
        self.counter.set(new_count);
        if let Some(cb) = &self.callback {
            cb(new_count);  // Safe because callback sees immutable self
        }
    }
}

// For multithreading, use Arc<Mutex<>> or Arc<RwLock<>>
use std::sync::{Arc, Mutex};

pub fn process_shared(data: Arc<Mutex<Vec<i32>>>) -> Result<(), ProcessError> {
    let mut guard = data.lock().map_err(|_| ProcessError::PoisonedLock)?;
    guard.push(42);
    Ok(())
}
```

**Pattern: Large Data Structures**

```rust
// For large data that needs interior mutability, use UnsafeCell sparingly:
use std::cell::UnsafeCell;

pub struct LargeBuffer {
    data: UnsafeCell<[u8; 1024 * 1024]>,
}

impl LargeBuffer {
    pub fn new() -> Self {
        LargeBuffer {
            data: UnsafeCell::new([0; 1024 * 1024]),
        }
    }
    
    pub fn get(&self) -> &[u8; 1024 * 1024] {
        // SAFETY: We never hand out mutable references to the same data
        unsafe { &*self.data.get() }
    }
    
    pub fn get_mut(&mut self) -> &mut [u8; 1024 * 1024] {
        // SAFETY: We have exclusive access via &mut self
        unsafe { &mut *self.data.get() }
    }
}
```

### 9. Minimize Raw Pointers

**Rule**: Prefer safe abstractions. Use raw pointers only when absolutely necessary for FFI or performance-critical hot paths, with clear safety comments.

**When to Use Raw Pointers:**

1. **FFI with C/C++ libraries**
   ```rust
   extern "C" {
       fn c_function(ptr: *const u8, len: usize) -> i32;
   }
   
   pub fn safe_wrapper(data: &[u8]) -> Result<i32, FfiError> {
       // Raw pointer needed for FFI, but wrapped safely
       let result = unsafe {
           c_function(data.as_ptr(), data.len())
       };
       if result < 0 {
           Err(FfiError::FunctionFailed(result))
       } else {
           Ok(result)
       }
   }
   ```

2. **Custom allocators (rare)**
   ```rust
   pub struct CustomVec<T> {
       ptr: *mut T,
       len: usize,
       capacity: usize,
   }
   
   impl<T> Drop for CustomVec<T> {
       fn drop(&mut self) {
           if self.capacity > 0 {
               // SAFETY: ptr was allocated with custom allocator and is valid
               unsafe {
                   std::alloc::dealloc(
                       self.ptr as *mut u8,
                       std::alloc::Layout::array::<T>(self.capacity).unwrap(),
                   );
               }
           }
       }
   }
   ```

**Prefer over Raw Pointers:**
- `&T` / `&mut T` for references
- `Box<T>` for owned heap allocation
- `Vec<T>` for dynamic arrays
- `String` for text
- `Option<T>` for nullable values
- `Rc<T>` / `Arc<T>` for shared ownership

## Refactoring Checklist

See `references/panic-to-result.md` for detailed examples of converting panic-prone C/C++ patterns to Rust `Result`-based error handling.

See `references/cpp-to-rust-patterns.md` for common C/C++ idioms and their Rust equivalents.

## Best Practices for C/C++ to Rust Conversion

- [ ] All pointer dereferences are wrapped in bounds/null checks or converted to safe types
- [ ] All integer operations that could overflow use `checked_*` or `saturating_*` methods
- [ ] Implicit enums (int constants) are converted to Rust enums
- [ ] Bit operations use `bitflags` crate instead of manual bit manipulation
- [ ] Pointer+size pairs are converted to slices
- [ ] Error codes are replaced with explicit error types using `thiserror`
- [ ] Invariants are impossible to violate due to type system constraints
- [ ] No unnecessary allocations; borrows used where appropriate
- [ ] `Cell`/`UnsafeCell` used only when borrow checker prevents safe code
- [ ] Raw pointers confined to FFI boundaries with clear safety justification
- [ ] All `unsafe` blocks have SAFETY comments explaining why it's sound
