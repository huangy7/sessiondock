# Contributing to SessionDock

Thank you for your interest in contributing to SessionDock! We welcome contributions from the community, whether it's reporting bugs, suggesting new features, or submitting pull requests.

## Reporting Bugs & Suggesting Features

If you encounter an issue or have an idea for a new feature, please [open an issue](https://github.com/huangy7/sessiondock/issues/new) on our GitHub repository. Before creating a new issue, please search existing issues to see if someone else has already reported it.

When reporting a bug, please include:
- A clear and descriptive title.
- Steps to reproduce the issue.
- Your operating system and SessionDock version.
- Any relevant logs or screenshots.

## Submitting Pull Requests

We gladly accept pull requests for bug fixes and new features. To submit a pull request:

1. Fork the repository and create your branch from `main`.
2. Ensure your local environment is set up (see [Local Environment Setup](#local-environment-setup)).
3. Make your changes, following our [Code Style Guidelines](#code-style-guidelines).
4. Test your changes thoroughly.
5. Commit your changes with a descriptive commit message (we recommend following the Conventional Commits specification).
6. Push your branch to your fork and submit a pull request to the `main` branch.

## Code Style Guidelines

- We use **TypeScript** for all frontend code. Please ensure your code is properly typed.
- We use **Prettier** and **ESLint** for code formatting and linting. Run `npm run lint` before submitting a PR.
- For **Rust** code in the backend (`src-tauri`), follow standard Rust formatting using `cargo fmt` and use `cargo clippy` to catch common mistakes.
- Write clear, concise comments for complex logic.

## Local Environment Setup

To set up your local development environment:

1. Ensure you have the following installed:
   - [Node.js](https://nodejs.org/) (v18 or newer recommended)
   - [Rust](https://www.rust-lang.org/) (latest stable version)
   - Tauri dependencies for your OS (see [Tauri Prerequisites](https://tauri.app/v1/guides/getting-started/prerequisites))

2. Clone your fork and install dependencies:
   ```bash
   git clone https://github.com/YOUR_USERNAME/sessiondock.git
   cd sessiondock
   npm install
   ```

3. Start the application in development mode:
   ```bash
   npm run dev
   ```

4. To run tests:
   ```bash
   npx vitest run
   ```

5. To verify the build:
   ```bash
   npm run build:web
   cd src-tauri
   cargo check
   ```

Thank you for contributing!
