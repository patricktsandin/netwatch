// ADR-0019: no `any`, no non-null assertions, and no `as` casts outside the boundary
// parser (src/api/parse.ts).
import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist/", "src/api/generated/"] },
  js.configs.recommended,
  ...tseslint.configs.strictTypeChecked,
  {
    languageOptions: {
      parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname },
    },
    rules: {
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/no-non-null-assertion": "error",
      "@typescript-eslint/consistent-type-assertions": ["error", { assertionStyle: "never" }],
      "@typescript-eslint/switch-exhaustiveness-check": "error",
    },
  },
  {
    files: ["src/api/parse.ts"],
    rules: { "@typescript-eslint/consistent-type-assertions": "off" },
  },
  {
    files: ["eslint.config.js"],
    ...tseslint.configs.disableTypeChecked,
  },
);
