# Common C/C++ to Rust Pattern Conversions

This reference provides side-by-side conversions of common C/C++ idioms to idiomatic Rust.

## Enums: Status Codes vs. Rust Errors

### C/C++ Status Codes

```c
#define STATUS_OK 0
#define STATUS_ERROR 1
#define STATUS_TIMEOUT 2
#define STATUS_INVALID_ARG 3

int process_file(const char* path) {
    FILE* f = fopen(path, "r");
    if (f == NULL) {
        return STATUS_INVALID_ARG;  // Wrong! Should be different error
    }
    
    if (fread(...) != expected) {
        fclose(f);
        return STATUS_ERROR;  // Vague!
    }
    
    fclose(f);
    return STATUS_OK;
}

int main() {
    int result = process_file("data.txt");
    if (result == STATUS_OK) {
        printf("Success\n");
    } else {
        printf("Error: %d\n", result);  // What does 2 mean?
    }
}
```

### Rust Result Type

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FileError {
    #[error("file not found: {path}")]
    NotFound { path: String },
    
    #[error("permission denied")]
    PermissionDenied,
    
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("read timeout")]
    Timeout,
    
    #[error("invalid argument: {reason}")]
    InvalidArgument { reason: String },
}

pub fn process_file(path: &str) -> Result<(), FileError> {
    let mut f = std::fs::File::open(path)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                FileError::NotFound { path: path.to_string() }
            }
            std::io::ErrorKind::PermissionDenied => FileError::PermissionDenied,
            _ => FileError::Io(e),
        })?;
    
    let mut buf = vec![0; 1024];
    f.read(&mut buf)?;  // Automatic ?-conversion
    
    Ok(())
}

fn main() -> Result<(), FileError> {
    process_file("data.txt")?;
    println!("Success");
    Ok(())
}
```

## Integer Flags: Magic Numbers vs. Bitflags

### C/C++ Bit Manipulation

```c
#define OPTION_VERBOSE 1
#define OPTION_DEBUG 2
#define OPTION_QUIET 4
#define OPTION_FORCE 8

typedef unsigned int Options;

void process(Options opts) {
    if (opts & OPTION_VERBOSE) {
        printf("Verbose mode\n");
    }
    
    opts |= OPTION_DEBUG;  // What if I typo this?
    opts &= ~OPTION_FORCE; // Easy to confuse with | and &
    
    // What are valid combinations?
    // Can VERBOSE and QUIET be set together?
    // No validation!
}

int main() {
    unsigned int opts = OPTION_VERBOSE | OPTION_DEBUG;
    process(opts);
    
    // Easy mistake:
    process(99);  // Invalid! No validation!
}
```

### Rust Bitflags

```rust
use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Options: u32 {
        const VERBOSE = 1;
        const DEBUG = 2;
        const QUIET = 4;
        const FORCE = 8;
    }
}

pub fn process(opts: Options) {
    if opts.contains(Options::VERBOSE) {
        println!("Verbose mode");
    }
    
    // Type-safe operations:
    let mut new_opts = opts;
    new_opts.insert(Options::DEBUG);
    new_opts.remove(Options::FORCE);
    
    // Iterate over set flags:
    for flag in opts.iter() {
        println!("Flag: {:?}", flag);
    }
}

fn main() {
    let opts = Options::VERBOSE | Options::DEBUG;
    process(opts);
    
    // Compiler enforces valid types
    // process(99);  // ERROR! Must be Options type
}
```

## Dynamic Arrays: Pointer+Size vs. Slices

### C/C++ Arrays

```c
// Takes pointer and size separately
void sort_array(int* arr, int n) {
    // Caller is responsible for synchronization
    // Easy to pass wrong size:
    // sort_array(ptr, wrong_size);  // UB!
}

// Returns pointer, size in output parameter
void allocate_buffer(uint8_t** out, int* out_len) {
    *out = malloc(256);
    *out_len = 256;
    // Caller might forget to free!
}

int main() {
    int arr[100];
    sort_array(arr, 100);
    
    uint8_t* buf;
    int len;
    allocate_buffer(&buf, &len);
    // ... use buf ...
    free(buf);  // Easy to forget!
}
```

### Rust Slices

```rust
// Size is part of the type
pub fn sort_array(arr: &mut [i32]) {
    arr.sort();  // Length is encoded in &mut [i32]
}

// Returns owned data, no freed needed
pub fn allocate_buffer() -> Vec<u8> {
    vec![0; 256]
}

fn main() {
    let mut arr = vec![3, 1, 4, 1, 5, 9];
    sort_array(&mut arr);  // Size automatic from reference
    
    let buf = allocate_buffer();
    // Vec<u8> automatically freed when dropped
    // No manual free() needed
}
```

## Validation: Manual Checks vs. Type Invariants

### C/C++ Runtime Validation

```c
typedef struct {
    char* email;
    int age;
} User;

// Validation scattered throughout
int is_valid_email(const char* email) {
    // Check if valid
    return 1;
}

User create_user(const char* email, int age) {
    User u;
    // Caller responsible for validation!
    // What if someone calls with invalid data?
    u.email = strdup(email);
    u.age = age;
    return u;
}

void use_user(User* u) {
    // Assume u->email is valid, but it might not be!
    // Validation is implicit contract, not enforced
    if (!is_valid_email(u->email)) {
        printf("Invalid email!\n");
        return;
    }
}
```

### Rust Type-System Validation

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ValidationError {
    #[error("invalid email format")]
    InvalidEmail,
    
    #[error("age must be 0-150")]
    InvalidAge { age: i32 },
}

// Newtype: Email is guaranteed valid
#[derive(Clone, Debug)]
pub struct Email(String);

impl Email {
    pub fn new(email: String) -> Result<Self, ValidationError> {
        if email.contains('@') && email.contains('.') {
            Ok(Email(email))
        } else {
            Err(ValidationError::InvalidEmail)
        }
    }
    
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct Age(u8);

impl Age {
    pub fn new(age: i32) -> Result<Self, ValidationError> {
        if age >= 0 && age <= 150 {
            Ok(Age(age as u8))
        } else {
            Err(ValidationError::InvalidAge { age })
        }
    }
    
    pub fn as_u8(&self) -> u8 {
        self.0
    }
}

pub struct User {
    email: Email,
    age: Age,
}

impl User {
    pub fn new(email: Email, age: Age) -> Self {
        User { email, age }
    }
}

fn use_user(u: &User) {
    // email is GUARANTEED valid here - type system ensures it
    println!("Email: {}", u.email.as_str());
}

fn main() -> Result<(), ValidationError> {
    let email = Email::new("user@example.com".to_string())?;
    let age = Age::new(30)?;
    let user = User::new(email, age);
    use_user(&user);
    Ok(())
}
```

## State Machines: Magic Numbers vs. Type States

### C/C++ State Machines

```c
// States as magic numbers
#define STATE_IDLE 0
#define STATE_CONNECTING 1
#define STATE_CONNECTED 2
#define STATE_DISCONNECTING 3

typedef struct {
    int state;
    int fd;
} Connection;

void connect(Connection* c) {
    // What if already connected?
    c->state = STATE_CONNECTING;
    c->fd = socket(...);
    c->state = STATE_CONNECTED;
}

void send_data(Connection* c, const char* data) {
    if (c->state == STATE_CONNECTED) {  // Magic number!
        write(c->fd, data, strlen(data));
    }
    // What if state is STATE_CONNECTING?
    // Race condition possible!
}

void disconnect(Connection* c) {
    // What if fd is invalid?
    close(c->fd);
    c->state = STATE_IDLE;
}
```

### Rust Type-Safe States

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConnectError {
    #[error("connection failed")]
    ConnectionFailed,
}

// State machine represented in types
pub enum Connection {
    Idle,
    Connecting,
    Connected(ConnectedSocket),
    Disconnecting,
}

pub struct ConnectedSocket {
    fd: i32,
}

impl ConnectedSocket {
    pub fn send(&mut self, data: &[u8]) -> std::io::Result<()> {
        // Can only send when Connected - impossible to call in wrong state!
        // fd is guaranteed valid
        std::os::unix::io::AsRawFd::as_raw_fd(self);
        Ok(())
    }
}

impl Connection {
    pub fn new() -> Self {
        Connection::Idle
    }
    
    pub fn connect(self) -> Result<Self, ConnectError> {
        match self {
            Connection::Idle => {
                let fd = 1;  // Simulate socket creation
                Ok(Connection::Connected(ConnectedSocket { fd }))
            }
            _ => Err(ConnectError::ConnectionFailed),
        }
    }
    
    pub fn disconnect(self) -> Self {
        match self {
            Connection::Connected(_) => {
                // Socket is automatically dropped and closed
                Connection::Idle
            }
            other => other,
        }
    }
}

fn main() -> Result<(), ConnectError> {
    let conn = Connection::new();
    let mut conn = conn.connect()?;
    
    // Type system guarantees conn is Connected here
    // match conn {
    //     Connection::Connected(ref mut sock) => {
    //         sock.send(b"hello")?;
    //     }
    //     _ => {}  // Can't reach here!
    // }
    
    let conn = conn.disconnect();
    Ok(())
}
```

## Ownership: Manual Freeing vs. RAII

### C/C++ Manual Memory Management

```c
typedef struct {
    char* data;
    int size;
} Buffer;

Buffer* create_buffer(int size) {
    Buffer* b = malloc(sizeof(Buffer));
    b->data = malloc(size);
    b->size = size;
    return b;
}

void free_buffer(Buffer* b) {
    free(b->data);
    free(b);
    // What if caller forgets to call this?
}

int main() {
    Buffer* buf = create_buffer(1024);
    // ... use buf ...
    free_buffer(buf);
    // If exception occurs before free_buffer, memory leaks!
}
```

### Rust RAII

```rust
pub struct Buffer {
    data: Vec<u8>,
}

impl Buffer {
    pub fn new(size: usize) -> Self {
        Buffer {
            data: vec![0; size],
        }
    }
}

fn main() {
    let buf = Buffer::new(1024);
    // ... use buf ...
    // Automatically freed when buf goes out of scope
    // Exception? Still freed!
}

// Or with Drop for complex cleanup:
impl Drop for Buffer {
    fn drop(&mut self) {
        // Cleanup code runs automatically
        println!("Cleaning up buffer");
    }
}
```

## String Handling: Manual Copying vs. Owned Strings

### C/C++ Strings

```c
#include <string.h>

// Returns malloc'd string - caller must free
char* format_message(const char* greeting, const char* name) {
    char* result = malloc(256);
    snprintf(result, 256, "%s, %s!", greeting, name);
    return result;
}

int main() {
    char* msg = format_message("Hello", "World");
    printf("%s\n", msg);
    free(msg);  // Easy to forget!
    
    // Or if using temporary:
    printf("%s\n", format_message("Hi", "Alice"));
    // Memory leaked! Message was never freed!
}
```

### Rust Strings

```rust
pub fn format_message(greeting: &str, name: &str) -> String {
    format!("{}, {}!", greeting, name)
}

fn main() {
    let msg = format_message("Hello", "World");
    println!("{}", msg);
    // Automatically freed when msg dropped
    
    // Or in-place without variable:
    println!("{}", format_message("Hi", "Alice"));
    // Automatically freed after println!
}

// If you need owned string with static lifetime:
pub fn format_message_static(greeting: &'static str, name: &'static str) -> &'static str {
    // This won't work for owned strings, use Box instead
    Box::leak(format!("{}, {}!", greeting, name).into_boxed_str())
}
```

## Bounds Checking: Explicit Checks vs. Type Safety

### C/C++ Bounds Checking

```c
// Manual bounds checking
int get_element(int* arr, int size, int index) {
    if (index < 0 || index >= size) {
        printf("Error: index out of bounds\n");
        return -1;  // Error code (but what if -1 is valid?)
    }
    return arr[index];
}

int main() {
    int arr[10] = {1, 2, 3};
    int x = get_element(arr, 10, 5);  // OK
    int y = get_element(arr, 5, 9);   // Wrong size passed! Bug!
}
```

### Rust Bounds Checking

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum IndexError {
    #[error("index {index} out of bounds for array of size {size}")]
    OutOfBounds { index: usize, size: usize },
}

pub fn get_element(arr: &[i32], index: usize) -> Result<i32, IndexError> {
    arr.get(index).copied().ok_or(IndexError::OutOfBounds {
        index,
        size: arr.len(),
    })
}

fn main() -> Result<(), IndexError> {
    let arr = [1, 2, 3];
    let x = get_element(&arr, 2)?;  // OK
    let y = get_element(&arr, 9)?;  // Returns error
    Ok(())
}
```
