# AGENTS.md - Open ScreenStudio

This document provides guidelines for AI coding agents working on this codebase.

## Project Overview

Open ScreenStudio is a Tauri 2.0 desktop application for professional screen recording.

- **Frontend**: React 18 + TypeScript + Vite + Tailwind CSS + Zustand
- **Backend**: Rust (Tauri 2.0)
- **Package Manager**: Bun (NOT npm/yarn)
- **Platforms**: macOS, Windows

## Build & Development Commands

```bash
# Frontend development (port 1420)
bun run dev

# Full Tauri development with hot reload
bun run tauri:dev

# Build frontend only
bun run build

# Build complete Tauri application
bun run tauri:build

# Linting
bun run lint                    # ESLint for TypeScript/React
cd src-tauri && cargo clippy    # Rust linting

# Formatting
bun run format                  # Prettier for frontend
cd src-tauri && cargo fmt       # Rust formatting

# Type checking
bun run build                   # Runs tsc && vite build
```

## Rust Backend Commands

```bash
cd src-tauri

# Build
cargo build
cargo build --release

# Run tests
cargo test                      # All tests
cargo test test_name            # Single test by name
cargo test module::             # Tests in a module

# Linting and formatting
cargo fmt --check               # Check formatting
cargo fmt                       # Apply formatting
cargo clippy                    # Lint with warnings
cargo clippy -- -D warnings     # Lint, treat warnings as errors
```

## Project Structure

```
open-screenstudio/
├── src/                        # React frontend
│   ├── components/             # React components (PascalCase files)
│   │   ├── editor/
│   │   ├── export/
│   │   └── recording/
│   ├── stores/                 # Zustand stores (camelCase files)
│   ├── types/                  # TypeScript type definitions
│   ├── App.tsx
│   └── main.tsx
├── src-tauri/                  # Rust backend
│   └── src/
│       ├── lib.rs              # App setup, command registration
│       ├── main.rs             # Binary entry point
│       ├── commands/           # Tauri IPC command handlers
│       ├── capture/            # Platform-specific capture code
│       │   ├── macos/
│       │   └── windows/
│       ├── recorder/           # Recording coordination
│       ├── project/            # Project file management
│       └── utils/              # Shared utilities, error types
└── docs/                       # Documentation
```

## TypeScript/React Code Style

### Import Order

```typescript
// 1. React imports
import { useState, useEffect } from "react";
// 2. Third-party libraries
import { Video, Edit3 } from "lucide-react";
// 3. Local components
import RecordingView from "./components/recording/RecordingView";
// 4. Stores
import { useProjectStore } from "./stores/projectStore";
// 5. Types (use 'type' keyword)
import type { Project, ProjectConfig } from "../types/project";
```

### Naming Conventions

- **Components**: PascalCase (`EditorView.tsx`)
- **Component files**: PascalCase (`RecordingView.tsx`)
- **Stores/utilities**: camelCase (`projectStore.ts`)
- **Types/Interfaces**: PascalCase (`ProjectConfig`)
- **Variables/Functions**: camelCase
- **Path alias**: `@/*` maps to `src/*`

### Component Patterns

```typescript
// Functional components with hooks
function MyComponent() {
  const [state, setState] = useState<string>("");
  // ...
  return <div>...</div>;
}

// Default export for components
export default MyComponent;

// Named export for stores
export const useMyStore = create<MyState>((set, get) => ({...}));
```

### Button Elements

Always add explicit `type="button"` to button elements:

```tsx
<button type="button" onClick={handleClick}>
  Click me
</button>
```

### State Management (Zustand)

```typescript
interface MyState {
  data: DataType | null;
  isLoading: boolean;
  error: string | null;
  // Actions
  fetchData: () => Promise<void>;
}

export const useMyStore = create<MyState>((set, get) => ({
  data: null,
  isLoading: false,
  error: null,
  fetchData: async () => {
    set({ isLoading: true, error: null });
    try {
      // ...
      set({ data: result, isLoading: false });
    } catch (e) {
      set({ error: "Error message", isLoading: false });
    }
  },
}));
```

## Rust Code Style

### Module Documentation

```rust
//! Module-level documentation
//!
//! Detailed description of the module's purpose.

pub mod submodule;

use crate::other_module;
use external_crate;
```

### Serde Serialization

Use `camelCase` for JSON compatibility with frontend:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MyStruct {
    pub field_name: String,  // Becomes fieldName in JSON
}
```

### Tauri Commands

```rust
#[tauri::command]
pub async fn my_command(
    state: State<'_, AppState>,
    param: ParamType,
) -> Result<ReturnType, String> {
    // Implementation
    // Convert errors to String for frontend
    operation().map_err(|e| e.to_string())
}
```

### Error Handling

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Custom error: {0}")]
    Custom(String),
}

pub type AppResult<T> = Result<T, AppError>;
```

### Platform-Specific Code

```rust
#[cfg(target_os = "macos")]
{
    // macOS-only code
}

#[cfg(target_os = "windows")]
{
    // Windows-only code
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
{
    // Fallback
}
```

## Commit Message Format

```
<type>: <short summary>

Types: feat, fix, docs, style, refactor, test, chore
```

Examples:

- `feat: Add automatic zoom detection for click events`
- `fix: Audio sync issue on Windows`
- `docs: Update installation instructions`

## Key Conventions

1. **TypeScript types mirror Rust schemas** - Keep `src/types/` in sync with `src-tauri/src/project/schema.rs`
2. **Use Bun** - Not npm or yarn
3. **Tailwind CSS** - Use CSS variables: `--background`, `--foreground`, `--muted`, `--accent`, `--border`
4. **Async/await** - Use `tokio` in Rust, standard async/await in TypeScript
5. **Logging** - Use `tracing` crate in Rust, `console.log` sparingly in TypeScript

## Testing Notes

- Frontend: No test framework currently configured
- Backend: Standard Cargo tests (`cargo test`)
- CI runs: lint, format check, clippy, build, and test on both platforms
