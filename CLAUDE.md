# Claude Development Guidelines

## Code Quality Workflow

When modifying code in this project, always follow these steps:

1. **Make code changes** - Edit the necessary files
2. **Format code** - Execute `npm run format` to format code with Prettier
3. **Run linting** - Execute `npm run lint` in the client directory to check for code style issues
4. **Fix any lint errors** - Address all ESLint warnings and errors
5. **Run type checking** - Execute `npm run check` to verify TypeScript types

## Common Commands

### Client Directory Commands

```bash
cd client
npm run lint          # Check for linting errors
npm run lint:fix       # Automatically fix linting errors
npm run format         # Format code with Prettier
npm run format:check   # Check if code is formatted correctly
npm run check          # Run Svelte and TypeScript type checking
npm run dev            # Start development server
npm run build          # Build for production
```

## Code Style Guidelines

- No semicolons (configured in Prettier)
- Use single quotes for strings
- Use proper TypeScript types (avoid `any`)
- Add keys to Svelte `{#each}` blocks
- Use block scopes in switch cases to avoid lexical declaration errors

## Notes

- ESLint is configured to work with JavaScript, TypeScript, and Svelte files
- Prettier is set up to format code without semicolons
- Always run lint checks after making code changes to maintain code quality
