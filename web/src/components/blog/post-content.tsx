"use client";

import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeRaw from "rehype-raw";

interface PostContentProps {
  content: string;
}

export function PostContent({ content }: PostContentProps) {
  return (
    <div
      className="prose prose-neutral dark:prose-invert max-w-none
        prose-headings:scroll-mt-20 prose-headings:font-semibold prose-headings:tracking-tight
        prose-h2:border-b prose-h2:pb-2 prose-h2:text-2xl
        prose-h3:text-xl
        prose-p:leading-7
        prose-a:text-primary prose-a:underline-offset-4 hover:prose-a:text-primary/80
        prose-img:rounded-lg prose-img:shadow-sm
        prose-blockquote:border-l-primary
        prose-code:rounded prose-code:bg-muted prose-code:px-1.5 prose-code:py-0.5 prose-code:text-sm
        prose-pre:bg-muted prose-pre:rounded-lg
        prose-li:marker:text-muted-foreground
        prose-table:text-sm
        prose-th:border-b prose-th:px-3 prose-th:py-2 prose-th:text-left
        prose-td:border-b prose-td:px-3 prose-td:py-2
        prose-video:max-w-full prose-video:rounded-lg
        prose-iframe:max-w-full prose-iframe:rounded-lg
        sm:prose-lg"
    >
      <ReactMarkdown remarkPlugins={[remarkGfm]} rehypePlugins={[rehypeRaw]}>
        {content}
      </ReactMarkdown>
    </div>
  );
}
