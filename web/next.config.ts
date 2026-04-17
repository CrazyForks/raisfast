import type { NextConfig } from "next";

const apiUrl = process.env.NEXT_PUBLIC_API_URL || "http://localhost:9000/api/v1";
const API_BASE = apiUrl.replace(/\/api\/v1$/, "");

let apiHost: string;
let apiPort: number;
let apiProtocol: string;
try {
  const url = new URL(API_BASE);
  apiHost = url.hostname;
  apiPort = parseInt(url.port) || (url.protocol === "https:" ? 443 : 80);
  apiProtocol = url.protocol.replace(":", "");
} catch {
  apiHost = "localhost";
  apiPort = 9000;
  apiProtocol = "http";
}

const nextConfig: NextConfig = {
  images: {
    remotePatterns: [
      {
        protocol: apiProtocol as "http" | "https",
        hostname: apiHost,
        port: apiPort.toString(),
      },
    ],
  },
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
