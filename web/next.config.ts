import type { NextConfig } from "next";

// A static export: `next build` writes `out/`, and `svanbot dashboard` serves it. No Node process
// runs on the fleet box (V11-11).
const config: NextConfig = {
  output: "export",
  trailingSlash: true,
};

export default config;
