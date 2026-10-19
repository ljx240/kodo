import { defineConfig } from "@playwright/test";
import base from "./playwright.config";

/** Isolated local server; never loads the workspace's environment files. */
export default defineConfig({
  ...base,
  testMatch: [/scheduler\.spec\.ts/, /page-polish\.spec\.ts/],
  projects: [{ name: "scheduler" }],
  reporter: "list",
  outputDir: "test-results/scheduler",
  use: { ...base.use, baseURL: "http://127.0.0.1:1442" },
  webServer: {
    command:
      'node --input-type=module -e \'import {createServer} from "vite"; const server = await createServer({envDir:false,server:{host:"127.0.0.1",port:1442,strictPort:true}}); await server.listen();\'',
    url: "http://127.0.0.1:1442",
    reuseExistingServer: false,
  },
});
