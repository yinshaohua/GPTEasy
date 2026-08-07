import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist", "node_modules", "src-tauri/target", "src-tauri/gen"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      globals: globals.browser,
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],
      "no-restricted-globals": [
        "error",
        { "name": "fetch", "message": "网络访问必须由 Rust 完整用例封装。" },
        { "name": "XMLHttpRequest", "message": "网络访问必须由 Rust 完整用例封装。" },
        { "name": "WebSocket", "message": "网络访问必须由 Rust 完整用例封装。" }
      ],
      "no-restricted-imports": [
        "error",
        {
          "patterns": [
            {
              "group": ["@tauri-apps/plugin-*"],
              "message": "前端不能直接获得文件、SQL、网络、Shell 或进程插件能力。"
            },
            {
              "group": ["node:*", "fs", "fs/*", "child_process", "net", "http", "https"],
              "message": "本地资源访问必须由 Rust 完整用例封装。"
            }
          ]
        }
      ]
    }
  }
);
