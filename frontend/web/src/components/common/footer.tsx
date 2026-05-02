import Link from "next/link";
import { Rss } from "lucide-react";

export function Footer() {
  return (
    <footer className="border-t">
      <div className="mx-auto flex max-w-5xl flex-col items-center gap-2 px-4 py-6 text-sm text-muted-foreground">
        <p>Built with Rust + Next.js</p>
        <Link
          href="/feed.xml"
          className="inline-flex items-center gap-1 hover:text-foreground"
          target="_blank"
          rel="noopener noreferrer"
        >
          <Rss className="size-3" />
          RSS Feed
        </Link>
      </div>
    </footer>
  );
}
