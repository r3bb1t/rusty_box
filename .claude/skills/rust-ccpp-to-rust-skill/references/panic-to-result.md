# Converting C/C++ Panics to Rust Results

This reference provides detailed, real-world examples of converting C/C++ code patterns that would panic in Rust into proper error handling.

## Pattern 1: Array Access and Bounds Checking

### C/C++ Code (Undefined Behavior)

```c
typedef struct {
    int data[100];
    int len;
} IntArray;

int get_element(IntArray* arr, int index) {
    // BUG: No bounds check! If index >= arr->len, undefined behavior
    return arr->data[index];
}

int main() {
    IntArray arr = {0};
    arr.len = 10;
    
    int x = get_element(&arr, 5);    // OK
    int y = get_element(&arr, 200);  // UB! Could crash, return garbage, or anything
}
```

### Rust Conversion

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ArrayError {
    #[error("index {index} out of bounds for array of length {len}")]
    IndexOutOfBounds { index: usize, len: usize },
}

pub struct IntArray {
    data: [i32; 100],
    len: usize,
}

impl IntArray {
    pub fn new() -> Self {
        IntArray {
            data: [0; 100],
            len: 0,
        }
    }
    
    // Safe version: cannot overflow, bounds are checked
    pub fn get(&self, index: usize) -> Result<i32, ArrayError> {
        if index < self.len {
            Ok(self.data[index])
        } else {
            Err(ArrayError::IndexOutOfBounds {
                index,
                len: self.len,
            })
        }
    }
    
    // Alternative: return slice, let caller use standard slice methods
    pub fn as_slice(&self) -> &[i32] {
        &self.data[..self.len]
    }
}

fn main() -> Result<(), ArrayError> {
    let mut arr = IntArray::new();
    arr.len = 10;
    
    let x = arr.get(5)?;      // OK
    let y = arr.get(200)?;    // Returns error immediately
    
    // Or use slice:
    let slice = arr.as_slice();
    if let Some(z) = slice.get(200) {
        println!("{}", z);
    }
    
    Ok(())
}
```

## Pattern 2: Pointer Dereferencing and Null Checks

### C/C++ Code (Null Pointer Dereference)

```c
typedef struct {
    char* name;
    int id;
} User;

const char* get_user_name(User* user) {
    // BUG: No null check on user. If user==NULL, UB!
    return user->name;
}

void process_user(User* user) {
    // Could also be NULL
    const char* name = get_user_name(user);
    printf("User: %s\n", name);  // BUG: name could be NULL too!
}
```

### Rust Conversion

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum UserError {
    #[error("user pointer was null")]
    NullUser,
    
    #[error("user name is missing")]
    MissingName,
}

pub struct User {
    name: Option<String>,
    id: u32,
}

impl User {
    pub fn get_name(&self) -> Result<&str, UserError> {
        self.name
            .as_ref()
            .map(|s| s.as_str())
            .ok_or(UserError::MissingName)
    }
}

// Function signature makes null-safety explicit
pub fn process_user(user: &User) -> Result<(), UserError> {
    let name = user.get_name()?;
    println!("User: {}", name);
    Ok(())
}

fn main() -> Result<(), UserError> {
    let user = User {
        name: Some("Alice".to_string()),
        id: 42,
    };
    
    process_user(&user)?;
    
    // Error case:
    let user_no_name = User {
        name: None,
        id: 43,
    };
    
    match process_user(&user_no_name) {
        Ok(_) => println!("OK"),
        Err(e) => eprintln!("Error: {}", e),
    }
    
    Ok(())
}
```

## Pattern 3: Integer Overflow

### C/C++ Code (Undefined Behavior)

```c
#include <limits.h>

int safe_add(int a, int b) {
    // BUG: If a + b overflows INT_MAX, result is undefined!
    int result = a + b;
    return result;
}

int main() {
    int x = INT_MAX;
    int y = 1;
    int z = safe_add(x, y);  // Overflow! UB!
    printf("%d\n", z);
}
```

### Rust Conversion

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MathError {
    #[error("integer overflow: {a} + {b}")]
    AddOverflow { a: i32, b: i32 },
    
    #[error("integer overflow: {a} * {b}")]
    MulOverflow { a: i32, b: i32 },
    
    #[error("integer underflow: {a} - {b}")]
    SubUnderflow { a: i32, b: i32 },
}

pub fn safe_add(a: i32, b: i32) -> Result<i32, MathError> {
    a.checked_add(b).ok_or(MathError::AddOverflow { a, b })
}

pub fn safe_mul(a: i32, b: i32) -> Result<i32, MathError> {
    a.checked_mul(b).ok_or(MathError::MulOverflow { a, b })
}

// Alternative: saturating arithmetic (no error, but limited range)
pub fn safe_add_saturating(a: i32, b: i32) -> i32 {
    a.saturating_add(b)  // Returns i32::MAX if overflow
}

fn main() -> Result<(), MathError> {
    let x = i32::MAX;
    let y = 1;
    let z = safe_add(x, y)?;  // Returns error
    println!("{}", z);
    Ok(())
}
```

## Pattern 4: String Parsing

### C/C++ Code (Silent Failure)

```c
#include <stdlib.h>
#include <string.h>

int parse_id(const char* str) {
    // BUG: atoi returns 0 for invalid input
    // Can't distinguish between "0" and "invalid"
    return atoi(str);
}

typedef struct {
    int id;
} Record;

Record create_record(const char* id_str) {
    Record rec;
    rec.id = parse_id(id_str);  // Silent failure if invalid!
    return rec;
}

int main() {
    Record r1 = create_record("42");        // OK
    Record r2 = create_record("invalid");   // r2.id = 0, silently!
}
```

### Rust Conversion

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("invalid integer '{input}': {reason}")]
    InvalidInteger { input: String, reason: String },
    
    #[error("value {value} out of expected range")]
    OutOfRange { value: String },
}

pub fn parse_id(s: &str) -> Result<u32, ParseError> {
    s.trim()
        .parse::<u32>()
        .map_err(|e| ParseError::InvalidInteger {
            input: s.to_string(),
            reason: e.to_string(),
        })
}

pub struct Record {
    id: u32,
}

impl Record {
    pub fn new(id_str: &str) -> Result<Self, ParseError> {
        let id = parse_id(id_str)?;
        Ok(Record { id })
    }
}

fn main() -> Result<(), ParseError> {
    let r1 = Record::new("42")?;       // OK
    let r2 = Record::new("invalid")?;  // Error returned immediately
    
    match Record::new("invalid") {
        Ok(r) => println!("ID: {}", r.id),
        Err(e) => eprintln!("Failed to parse: {}", e),
    }
    
    Ok(())
}
```

## Pattern 5: Collection Element Access

### C/C++ Code (Buffer Overflow)

```c
#include <string.h>

typedef struct {
    char items[10][256];
    int count;
} StringList;

const char* get_item(StringList* list, int index) {
    // BUG: No bounds check!
    return list->items[index];
}

void add_item(StringList* list, const char* item) {
    // BUG: No overflow check on count!
    strcpy(list->items[list->count], item);
    list->count++;
}

int main() {
    StringList list = {0};
    add_item(&list, "hello");
    
    const char* x = get_item(&list, 100);  // UB! Out of bounds!
}
```

### Rust Conversion

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ListError {
    #[error("index {index} out of bounds for list of size {size}")]
    IndexOutOfBounds { index: usize, size: usize },
    
    #[error("list full: maximum {max} items")]
    ListFull { max: usize },
    
    #[error("string too long: max {max} bytes")]
    StringTooLong { max: usize },
}

pub struct StringList {
    items: Vec<String>,
    max_items: usize,
    max_item_len: usize,
}

impl StringList {
    pub fn new(max_items: usize, max_item_len: usize) -> Self {
        StringList {
            items: Vec::new(),
            max_items,
            max_item_len,
        }
    }
    
    pub fn get(&self, index: usize) -> Result<&str, ListError> {
        self.items
            .get(index)
            .map(|s| s.as_str())
            .ok_or(ListError::IndexOutOfBounds {
                index,
                size: self.items.len(),
            })
    }
    
    pub fn add(&mut self, item: &str) -> Result<(), ListError> {
        if self.items.len() >= self.max_items {
            return Err(ListError::ListFull {
                max: self.max_items,
            });
        }
        
        if item.len() > self.max_item_len {
            return Err(ListError::StringTooLong {
                max: self.max_item_len,
            });
        }
        
        self.items.push(item.to_string());
        Ok(())
    }
}

fn main() -> Result<(), ListError> {
    let mut list = StringList::new(10, 256);
    list.add("hello")?;
    
    let x = list.get(0)?;
    println!("{}", x);
    
    // Error cases are explicit:
    match list.get(100) {
        Ok(s) => println!("{}", s),
        Err(e) => eprintln!("Error: {}", e),
    }
    
    Ok(())
}
```

## Pattern 6: Option to Result Conversions

### C/C++ Code (Find with Error Handling)

```c
#include <stdlib.h>

// Returns NULL if not found - caller must check
int* find_user(User* users, int count, int id) {
    for (int i = 0; i < count; i++) {
        if (users[i].id == id) {
            return &users[i];
        }
    }
    return NULL;  // Caller must check!
}

void process_user(User* users, int count, int id) {
    User* user = find_user(users, count, id);
    if (user == NULL) {
        printf("Not found\n");
        return;
    }
    printf("Found: %s\n", user->name);
}
```

### Rust Conversion

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum UserError {
    #[error("user with id {id} not found")]
    NotFound { id: u32 },
}

pub struct User {
    id: u32,
    name: String,
}

impl User {
    pub fn find(users: &[User], id: u32) -> Result<&User, UserError> {
        users
            .iter()
            .find(|u| u.id == id)
            .ok_or(UserError::NotFound { id })
    }
    
    // With mut borrow:
    pub fn find_mut(users: &mut [User], id: u32) -> Result<&mut User, UserError> {
        users
            .iter_mut()
            .find(|u| u.id == id)
            .ok_or(UserError::NotFound { id })
    }
}

fn process_user(users: &[User], id: u32) -> Result<(), UserError> {
    let user = User::find(users, id)?;
    println!("Found: {}", user.name);
    Ok(())
}

fn main() -> Result<(), UserError> {
    let users = vec![
        User {
            id: 1,
            name: "Alice".to_string(),
        },
        User {
            id: 2,
            name: "Bob".to_string(),
        },
    ];
    
    process_user(&users, 1)?;
    
    match process_user(&users, 999) {
        Ok(_) => println!("OK"),
        Err(e) => eprintln!("Error: {}", e),
    }
    
    Ok(())
}
```

## Pattern 7: Checked Index Operations in Loops

### C/C++ Code (Potential Overflow)

```c
void process_all(int* data, int len) {
    for (int i = 0; i < len; i++) {
        // BUG: If len == INT_MAX, i++ could overflow
        process(data[i]);
    }
}
```

### Rust Conversion

```rust
pub fn process_all(data: &[i32]) {
    for &item in data {
        process(item);  // No index, no overflow possible
    }
}

// Or with enumerate if you need index:
pub fn process_all_with_index(data: &[i32]) {
    for (i, &item) in data.iter().enumerate() {
        println!("Index {}: {}", i, item);
        process(item);
    }
    // enumerate uses usize, which can't overflow the slice length
}
```
