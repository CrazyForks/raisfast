import type { NextConfig } from "next";

const apiUrl = process.env.NEXT_PUBLIC_API_URL || "http://localhost:9000/api/v1";
const API_BASE = apiUrl.replace(/\/api\/v1$/, "");

const nextConfig: NextConfig = {
  async rewrites() {
    return [
      {
        source: "/feed.xml",
        destination: `${API_BASE}/feed.xml`,
      },
    ];
  },
};

export default nextConfig;
