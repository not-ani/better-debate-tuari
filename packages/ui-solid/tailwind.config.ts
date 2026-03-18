import type { Config } from "tailwindcss";

const config: Config = {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      fontFamily: {
        sans: ["'DM Sans'", "system-ui", "-apple-system", "sans-serif"],
        mono: ["'DM Mono'", "ui-monospace", "monospace"],
      },
      colors: {
        surface: {
          0: "var(--surface-0)",
          1: "var(--surface-1)",
          2: "var(--surface-2)",
          3: "var(--surface-3)",
          4: "var(--surface-4)",
        },
        accent: {
          DEFAULT: "var(--accent)",
          dim: "var(--accent-dim)",
          subtle: "var(--accent-subtle)",
          fg: "var(--accent-fg)",
          bright: "var(--accent-bright)",
        },
      },
      borderColor: {
        dim: "var(--border-dim)",
        subtle: "var(--border-subtle)",
        default: "var(--border-default)",
        strong: "var(--border-strong)",
      },
      textColor: {
        primary: "var(--text-primary)",
        secondary: "var(--text-secondary)",
        tertiary: "var(--text-tertiary)",
        ghost: "var(--text-ghost)",
      },
      fontSize: {
        "2xs": ["11px", { lineHeight: "16px" }],
      },
      screens: {
        xs: "420px",
        sm: "540px",
        md: "720px",
        lg: "960px",
        xl: "1200px",
      },
    },
  },
  plugins: [],
};

export default config;
