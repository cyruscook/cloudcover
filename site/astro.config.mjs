import { defineConfig } from "astro/config";

export default defineConfig({
  output: "static",
  site: "https://cyruscook.github.io",
  base: "/cloudcover",
  trailingSlash: "always",
});
