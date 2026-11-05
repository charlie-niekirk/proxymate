import js from "@eslint/js";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import { defineConfig, globalIgnores } from "eslint/config";
import globals from "globals";
import jsxA11y from "eslint-plugin-jsx-a11y-x";
import tseslint from "typescript-eslint";

export default defineConfig([
  globalIgnores(["dist", "src/bindings.ts"]),
  {
    files: ["**/*.{ts,tsx}"],
    extends: [
      js.configs.recommended,
      tseslint.configs.recommended,
      reactHooks.configs.flat["recommended-latest"],
      reactRefresh.configs.vite,
    ],
    languageOptions: {
      ecmaVersion: 2023,
      globals: globals.browser,
    },
  },
  {
    // Type-aware: a promise nobody awaits, returns or `void`s is an error
    // that would vanish (a rejected IPC call with no toast).
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: { parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname } },
    rules: { "@typescript-eslint/no-floating-promises": "error" },
  },
  {
    // R4: accessibility. The jsx-a11y-x fork is the upstream rule set with
    // ESLint 10 support (eslint-plugin-jsx-a11y's peer range stops at 9).
    files: ["src/**/*.tsx"],
    extends: [jsxA11y.configs.recommended],
    rules: {
      // Our switches, selects and inputs are labelable controls inside a <label>.
      "jsx-a11y-x/label-has-associated-control": [
        "error",
        { assert: "either", controlComponents: ["Switch", "Select", "Input", "PathInput"], depth: 4 },
      ],
    },
  },
  {
    // shadcn/ui components export variants next to components by design.
    files: ["src/components/ui/**"],
    rules: { "react-refresh/only-export-components": "off" },
  },
]);
