import TurndownService from "turndown";
import { marked } from "marked";
import { gfm } from "turndown-plugin-gfm";

marked.use({ gfm: true, breaks: true });

const turndown = new TurndownService({
  headingStyle: "atx",
  codeBlockStyle: "fenced",
  bulletListMarker: "-",
});

turndown.use(gfm);

export function htmlToMarkdown(html: string): string {
  return turndown.turndown(html);
}

export function markdownToHtml(md: string): string {
  return marked.parse(md) as string;
}
