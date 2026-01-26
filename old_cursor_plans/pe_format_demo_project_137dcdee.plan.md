---
name: PE Format Demo Project
overview: "Create a CMake-based C++ project with two executables demonstrating PE format: one using LoadLibrary/GetProcAddress, and one with fully manual PE parsing using VirtualAlloc and manual import resolution to call MessageBox."
todos:
  - id: cmake_setup
    content: Create CMakeLists.txt with C++20 configuration and two executable targets, each in its own folder
    status: completed
  - id: loadlibrary_impl
    content: Implement loadlibrary_demo/main.cpp using LoadLibrary and GetProcAddress
    status: completed
  - id: manual_pe_impl
    content: Implement manual_pe_demo/main.cpp with full PE parsing (headers, sections, imports, IAT resolution)
    status: completed
  - id: readme
    content: Create README.md with project description and build instructions
    status: completed
---

# PE Format Demonstration Project

A modern C++ project demonstrating Windows PE (Portable Executable) format by manually loading DLLs and resolving imports to call MessageBox.

## Project Structure

```
.
├── CMakeLists.txt
├── loadlibrary_demo/
│   ├── main.cpp                # Standard Windows API approach
│   └── (auxiliary files can be placed here)
├── manual_pe_demo/
│   ├── main.cpp                # Fully manual PE parsing
│   └── (auxiliary files can be placed here)
└── README.md
```

## Implementation Details

### 1. CMakeLists.txt

- Configure CMake for C++20
- Build two separate executables: `loadlibrary_demo` and `manual_pe_demo`
- Each executable built from its respective folder's `main.cpp`
- Set appropriate Windows-specific flags and link libraries

### 2. loadlibrary_demo/main.cpp

Simple demonstration using standard Windows APIs:

- Use `LoadLibraryA` to load `user32.dll`
- Use `GetProcAddress` to get `MessageBoxA` function pointer
- Call MessageBox with "Hello World" message
- Include clear comments explaining each step

### 3. manual_pe_demo/main.cpp

Fully manual PE parsing implementation:

- **Read DLL from disk**: Open and read `user32.dll` file
- **Parse PE Headers**:
  - DOS header (MZ signature)
  - NT headers (PE signature, FileHeader, OptionalHeader)
  - Section headers
- **Memory Allocation**: Use `VirtualAlloc` with `PAGE_EXECUTE_READWRITE` to allocate executable memory
- **Copy Sections**: Map sections to allocated memory respecting virtual addresses
- **Process Relocations**: Handle base address relocations if needed
- **Resolve Imports Manually**:
  - Parse Import Directory Table
  - For each imported DLL, recursively load it (or use LoadLibrary for system DLLs)
  - Resolve function addresses by name or ordinal
  - Fill Import Address Table (IAT)
- **Call MessageBox**: Get function pointer from IAT and call with "Hello World"
- Include extensive comments explaining PE structure at each step

### 4. README.md

- Project description
- Build instructions
- Explanation of both approaches
- PE format concepts covered

## Key PE Format Concepts Demonstrated

- DOS header and MZ signature
- NT headers (PE signature, COFF header, Optional header)
- Section headers and section alignment
- Import Directory Table (IDT)
- Import Address Table (IAT)
- Base relocations
- Virtual memory mapping

## Code Style

- Modern C++20 features where appropriate
- Clear variable names and comments
- Step-by-step explanations in comments
- Error handling with meaningful messages
- Well-structured functions for each PE parsing step