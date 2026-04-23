"use client";

import { useState, useEffect } from "react";

export function BlockRenderer({ blocks }: { blocks: Record<string, unknown>[] }) {
  if (!blocks?.length) return null;

  return (
    <div className="space-y-0">
      {blocks.map((block, idx) => (
        <Block key={idx} block={block} />
      ))}
    </div>
  );
}

function Block({ block }: { block: Record<string, unknown> }) {
  const type = block.type as string;

  switch (type) {
    case "hero":
      return <HeroBlock block={block} />;
    case "richtext":
      return <RichtextBlock block={block} />;
    case "image":
      return <ImageBlock block={block} />;
    case "gallery":
      return <GalleryBlock block={block} />;
    case "video":
      return <VideoBlock block={block} />;
    case "cta":
      return <CtaBlock block={block} />;
    case "stats":
      return <StatsBlock block={block} />;
    case "faq":
      return <FaqBlock block={block} />;
    case "timeline":
      return <TimelineBlock block={block} />;
    case "team":
      return <TeamBlock block={block} />;
    case "pricing":
      return <PricingBlock block={block} />;
    case "contact_form":
      return <ContactFormBlock block={block} />;
    case "quote":
      return <QuoteBlock block={block} />;
    case "code":
      return <CodeBlock block={block} />;
    case "divider":
      return <DividerBlock block={block} />;
    case "spacer":
      return <SpacerBlock block={block} />;
    case "html":
      return <HtmlBlock block={block} />;
    case "testimonial":
      return <TestimonialBlock block={block} />;
    case "map":
      return <MapBlock block={block} />;
    case "columns":
      return <ColumnsBlock block={block} />;
    case "reusable":
      return <ReusableBlockRenderer block={block} />;
    default:
      return null;
  }
}

function HeroBlock({ block }: { block: Record<string, unknown> }) {
  const heightClass: Record<string, string> = { sm: "min-h-[300px]", md: "min-h-[450px]", lg: "min-h-[600px]", full: "min-h-screen" };
  const h = (block.height as string) ?? "md";
  return (
    <section
      className={`relative flex items-center justify-center ${heightClass[h] ?? heightClass.md} bg-gradient-to-br from-primary/10 to-primary/5`}
      style={block.background_image ? { backgroundImage: `url(${block.background_image})`, backgroundSize: "cover", backgroundPosition: "center" } : undefined}
    >
      {block.overlay === true && <div className="absolute inset-0 bg-black/40" />}
      <div className={`relative z-10 text-center max-w-3xl mx-auto px-6 ${block.overlay ? "text-white" : ""}`}>
        <h1 className="text-4xl md:text-5xl font-bold mb-4">{block.title as string}</h1>
        {block.subtitle ? <p className="text-xl text-current/80 mb-8">{block.subtitle as string}</p> : null}
        {block.cta_text ? (
          <a href={block.cta_url as string} className="inline-flex items-center justify-center rounded-md bg-primary text-primary-foreground px-8 py-3 text-lg font-medium hover:bg-primary/90">
            {block.cta_text as string}
          </a>
        ) : null}
      </div>
    </section>
  );
}

function RichtextBlock({ block }: { block: Record<string, unknown> }) {
  return (
    <section className="max-w-3xl mx-auto px-6 py-12">
      <div className="prose prose-lg dark:prose-invert max-w-none whitespace-pre-wrap">{block.content as string}</div>
    </section>
  );
}

function ImageBlock({ block }: { block: Record<string, unknown> }) {
  const widthClass: Record<string, string> = { full: "w-full", half: "w-1/2", third: "w-1/3", quarter: "w-1/4" };
  const w = (block.width as string) ?? "full";
  return (
    <section className="max-w-4xl mx-auto px-6 py-8">
      <div className={widthClass[w] ?? "w-full"} mx-auto>
        <img src={block.url as string} alt={(block.alt as string) ?? ""} className="rounded-lg w-full" />
        {block.caption ? <p className="text-sm text-muted-foreground text-center mt-2">{block.caption as string}</p> : null}
      </div>
    </section>
  );
}

function GalleryBlock({ block }: { block: Record<string, unknown> }) {
  const images = (block.images as { url: string; alt?: string; caption?: string }[]) ?? [];
  const cols = (block.columns as number) ?? 3;
  return (
    <section className="max-w-5xl mx-auto px-6 py-12">
      <div className={`grid gap-4 grid-cols-${cols}`}>
        {images.map((img, i) => (
          <div key={i}>
            <img src={img.url} alt={img.alt ?? ""} className="rounded-lg w-full object-cover aspect-[4/3]" />
            {img.caption && <p className="text-sm text-muted-foreground text-center mt-1">{img.caption}</p>}
          </div>
        ))}
      </div>
    </section>
  );
}

function VideoBlock({ block }: { block: Record<string, unknown> }) {
  const url = block.url as string;
  let embedUrl = url;
  if (url?.includes("youtube.com/watch")) {
    const id = new URL(url).searchParams.get("v");
    embedUrl = `https://www.youtube.com/embed/${id}`;
  } else if (url?.includes("youtu.be/")) {
    embedUrl = `https://www.youtube.com/embed/${url.split("youtu.be/")[1]}`;
  } else if (url?.includes("bilibili.com/video/")) {
    const bvid = url.split("bilibili.com/video/")[1]?.split("?")[0];
    embedUrl = `https://player.bilibili.com/player.html?bvid=${bvid}&autoplay=0`;
  }
  return (
    <section className="max-w-4xl mx-auto px-6 py-12">
      <div className="aspect-video rounded-lg overflow-hidden">
        <iframe src={embedUrl} className="w-full h-full" allowFullScreen title={block.title as string ?? "Video"} />
      </div>
    </section>
  );
}

function CtaBlock({ block }: { block: Record<string, unknown> }) {
  return (
    <section className="bg-primary/5 py-16">
      <div className="max-w-3xl mx-auto text-center px-6">
        <h2 className="text-3xl font-bold mb-4">{block.title as string}</h2>
        {block.description ? <p className="text-lg text-muted-foreground mb-8">{block.description as string}</p> : null}
        <a href={block.button_url as string} className="inline-flex items-center justify-center rounded-md bg-primary text-primary-foreground px-8 py-3 text-lg font-medium hover:bg-primary/90">
          {block.button_text as string}
        </a>
      </div>
    </section>
  );
}

function StatsBlock({ block }: { block: Record<string, unknown> }) {
  const items = (block.items as { label: string; value: string; suffix?: string; description?: string }[]) ?? [];
  return (
    <section className="py-16">
      <div className="max-w-5xl mx-auto px-6 grid grid-cols-2 md:grid-cols-4 gap-8">
        {items.map((item, i) => (
          <div key={i} className="text-center">
            <div className="text-4xl font-bold text-primary">{item.value}{item.suffix ?? ""}</div>
            <div className="text-sm text-muted-foreground mt-1">{item.label}</div>
            {item.description && <div className="text-xs text-muted-foreground mt-0.5">{item.description}</div>}
          </div>
        ))}
      </div>
    </section>
  );
}

function FaqBlock({ block }: { block: Record<string, unknown> }) {
  const items = (block.items as { question: string; answer: string }[]) ?? [];
  return (
    <section className="max-w-3xl mx-auto px-6 py-12">
      <div className="space-y-4">
        {items.map((item, i) => (
          <details key={i} className="rounded-lg border p-4 group">
            <summary className="font-medium cursor-pointer list-none flex items-center justify-between">
              {item.question}
              <span className="text-muted-foreground group-open:rotate-180 transition-transform">▼</span>
            </summary>
            <p className="mt-3 text-muted-foreground">{item.answer}</p>
          </details>
        ))}
      </div>
    </section>
  );
}

function TimelineBlock({ block }: { block: Record<string, unknown> }) {
  const items = (block.items as { date: string; title: string; description?: string }[]) ?? [];
  return (
    <section className="max-w-3xl mx-auto px-6 py-12">
      <div className="relative">
        <div className="absolute left-4 top-0 bottom-0 w-0.5 bg-border" />
        <div className="space-y-8">
          {items.map((item, i) => (
            <div key={i} className="relative pl-10">
              <div className="absolute left-2.5 top-1 size-3 rounded-full bg-primary" />
              <div className="text-sm text-muted-foreground">{item.date}</div>
              <h3 className="font-medium mt-1">{item.title}</h3>
              {item.description && <p className="text-sm text-muted-foreground mt-1">{item.description}</p>}
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

function TeamBlock({ block }: { block: Record<string, unknown> }) {
  const members = (block.members as { name: string; role?: string; avatar?: string; bio?: string }[]) ?? [];
  const cols = (block.columns as number) ?? 4;
  return (
    <section className="max-w-5xl mx-auto px-6 py-12">
      <div className={`grid gap-8 md:grid-cols-${cols}`}>
        {members.map((m, i) => (
          <div key={i} className="text-center">
            {m.avatar ? (
              <img src={m.avatar} alt={m.name} className="size-20 rounded-full mx-auto mb-3 object-cover" />
            ) : (
              <div className="size-20 rounded-full bg-muted mx-auto mb-3 flex items-center justify-center text-2xl font-medium">{m.name.charAt(0)}</div>
            )}
            <h3 className="font-medium">{m.name}</h3>
            {m.role && <p className="text-sm text-muted-foreground">{m.role}</p>}
            {m.bio && <p className="text-xs text-muted-foreground mt-2">{m.bio}</p>}
          </div>
        ))}
      </div>
    </section>
  );
}

function PricingBlock({ block }: { block: Record<string, unknown> }) {
  const plans = (block.plans as { name: string; price: string; period?: string; description?: string; features: string[]; button_text?: string; button_url?: string }[]) ?? [];
  const highlight = block.highlight_index as number | undefined;
  return (
    <section className="max-w-5xl mx-auto px-6 py-12">
      <div className="grid md:grid-cols-3 gap-6">
        {plans.map((plan, i) => (
          <div key={i} className={`rounded-lg border p-6 ${i === highlight ? "border-primary shadow-lg" : ""}`}>
            <h3 className="text-lg font-semibold">{plan.name}</h3>
            <div className="mt-4 mb-6">
              <span className="text-4xl font-bold">{plan.price}</span>
              {plan.period && <span className="text-muted-foreground">/{plan.period}</span>}
            </div>
            {plan.description && <p className="text-sm text-muted-foreground mb-4">{plan.description}</p>}
            <ul className="space-y-2 mb-6">
              {plan.features.map((f, j) => (
                <li key={j} className="text-sm flex items-center gap-2"><span className="text-primary">✓</span>{f}</li>
              ))}
            </ul>
            {plan.button_text && (
              <a href={plan.button_url ?? "#"} className={`block text-center rounded-md py-2 text-sm font-medium ${i === highlight ? "bg-primary text-primary-foreground" : "border"}`}>
                {plan.button_text}
              </a>
            )}
          </div>
        ))}
      </div>
    </section>
  );
}

function ContactFormBlock({ block }: { block: Record<string, unknown> }) {
  const fields = (block.fields as { name: string; label: string; field_type: string; required?: boolean }[]) ?? [
    { name: "name", label: "Name", field_type: "text", required: true },
    { name: "email", label: "Email", field_type: "email", required: true },
    { name: "message", label: "Message", field_type: "textarea", required: true },
  ];
  return (
    <section className="max-w-xl mx-auto px-6 py-12">
      <form className="space-y-4" onSubmit={(e) => e.preventDefault()}>
        {fields.map((f, i) => (
          <div key={i}>
            <label className="block text-sm font-medium mb-1">{f.label}{f.required ? " *" : ""}</label>
            {f.field_type === "textarea" ? (
              <textarea className="flex min-h-[100px] w-full rounded-md border border-input bg-background px-3 py-2 text-sm" name={f.name} required={f.required} />
            ) : (
              <input className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm" type={f.field_type} name={f.name} required={f.required} />
            )}
          </div>
        ))}
        <button type="submit" className="w-full rounded-md bg-primary text-primary-foreground py-2 text-sm font-medium hover:bg-primary/90">
          {(block.submit_text as string) ?? "Send Message"}
        </button>
      </form>
    </section>
  );
}

function QuoteBlock({ block }: { block: Record<string, unknown> }) {
  return (
    <section className="max-w-3xl mx-auto px-6 py-12">
      <blockquote className="border-l-4 border-primary pl-6 py-2">
        <p className="text-xl italic text-muted-foreground">"{block.content as string}"</p>
        {block.author ? <footer className="mt-3 text-sm font-medium">— {block.author as string}{block.source ? <span className="text-muted-foreground">, {block.source as string}</span> : null}</footer> : null}
      </blockquote>
    </section>
  );
}

function CodeBlock({ block }: { block: Record<string, unknown> }) {
  return (
    <section className="max-w-4xl mx-auto px-6 py-8">
      <pre className="rounded-lg bg-muted p-4 overflow-x-auto text-sm">
        {block.language ? <div className="text-xs text-muted-foreground mb-2">{block.language as string}</div> : null}
        <code>{block.code as string}</code>
      </pre>
    </section>
  );
}

function DividerBlock({ block }: { block: Record<string, unknown> }) {
  const style = (block.style as string) ?? "solid";
  if (style === "space") return <div className="py-8" />;
  const borderStyle = style === "dashed" ? "border-dashed" : style === "dotted" ? "border-dotted" : "border-solid";
  return <hr className={`max-w-xl mx-auto my-8 border-t ${borderStyle}`} />;
}

function SpacerBlock({ block }: { block: Record<string, unknown> }) {
  const heightMap: Record<string, string> = { sm: "2rem", md: "4rem", lg: "6rem", xl: "8rem" };
  const h = (block.height as string) ?? "md";
  return <div style={{ height: heightMap[h] ?? h }} />;
}

function HtmlBlock({ block }: { block: Record<string, unknown> }) {
  return (
    <section className="max-w-5xl mx-auto px-6 py-8" dangerouslySetInnerHTML={{ __html: (block.content as string) ?? "" }} />
  );
}

function TestimonialBlock({ block }: { block: Record<string, unknown> }) {
  const items = (block.items as { quote: string; author: string; company?: string; avatar?: string; rating?: number }[]) ?? [];
  return (
    <section className="max-w-5xl mx-auto px-6 py-12">
      <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-6">
        {items.map((item, i) => (
          <div key={i} className="rounded-lg border p-6">
            {item.rating && (
              <div className="flex gap-0.5 mb-3">
                {Array.from({ length: Math.min(item.rating, 5) }, (_, j) => (
                  <span key={j} className="text-yellow-500">★</span>
                ))}
              </div>
            )}
            <p className="text-sm text-muted-foreground mb-4">"{item.quote}"</p>
            <div className="flex items-center gap-3">
              {item.avatar ? (
                <img src={item.avatar} alt={item.author} className="size-10 rounded-full object-cover" />
              ) : (
                <div className="size-10 rounded-full bg-muted flex items-center justify-center text-sm font-medium">{item.author?.charAt(0) ?? "?"}</div>
              )}
              <div>
                <div className="text-sm font-medium">{item.author}</div>
                {item.company && <div className="text-xs text-muted-foreground">{item.company}</div>}
              </div>
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}

function MapBlock({ block }: { block: Record<string, unknown> }) {
  const lat = parseFloat(String(block.lat ?? "0")) || 0;
  const lng = parseFloat(String(block.lng ?? "0")) || 0;
  const zoom = parseInt(String(block.zoom ?? "14")) || 14;
  const address = block.address as string;
  const title = (block.title as string) ?? "Map";

  if (lat === 0 && lng === 0 && !address) {
    return (
      <section className="max-w-5xl mx-auto px-6 py-8">
        <div className="rounded-lg bg-muted flex items-center justify-center min-h-[300px] text-muted-foreground text-sm">
          Map: configure address or coordinates
        </div>
      </section>
    );
  }

  const embedQuery = address
    ? encodeURIComponent(address)
    : `${lat},${lng}`;
  const embedUrl = `https://www.openstreetmap.org/export/embed.html?bbox=${lng - 0.01}%2C${lat - 0.01}%2C${lng + 0.01}%2C${lat + 0.01}&layer=mapnik&marker=${lat}%2C${lng}`;
  const searchUrl = `https://www.openstreetmap.org/search?query=${embedQuery}`;

  return (
    <section className="max-w-5xl mx-auto px-6 py-8">
      <div className="rounded-lg overflow-hidden border">
        <iframe
          src={embedUrl}
          className="w-full border-0"
          style={{ height: "400px" }}
          loading="lazy"
          title={title}
        />
      </div>
      {address ? (
        <p className="text-sm text-muted-foreground text-center mt-2">
          <a href={searchUrl} target="_blank" rel="noopener noreferrer" className="hover:underline">{address}</a>
        </p>
      ) : null}
    </section>
  );
}

function ColumnsBlock({ block }: { block: Record<string, unknown> }) {
  const columns = (block.columns as { blocks: Record<string, unknown>[] }[]) ?? [];
  if (!columns.length) return null;

  const colWidthClass: Record<number, string> = {
    1: "grid-cols-1",
    2: "grid-cols-1 md:grid-cols-2",
    3: "grid-cols-1 md:grid-cols-3",
    4: "grid-cols-1 md:grid-cols-2 lg:grid-cols-4",
    5: "grid-cols-1 md:grid-cols-5",
    6: "grid-cols-1 md:grid-cols-2 lg:grid-cols-6",
  };
  const cls = colWidthClass[columns.length] ?? `grid-cols-1 md:grid-cols-${columns.length}`;

  return (
    <section className="max-w-5xl mx-auto px-6 py-8">
      <div className={`grid gap-6 ${cls}`}>
        {columns.map((col, i) => (
          <div key={i}>
            {col.blocks?.length ? <BlockRenderer blocks={col.blocks} /> : null}
          </div>
        ))}
      </div>
    </section>
  );
}

function ReusableBlockRenderer({ block }: { block: Record<string, unknown> }) {
  const refId = block.ref_id as string;
  const [content, setContent] = useState<Record<string, unknown> | null>(null);

  useEffect(() => {
    if (!refId) return;
    import("@/lib/page").then(({ page }) => {
      page.listReusable().then((blocks) => {
        const rb = blocks.find((b) => b.id === refId);
        if (rb) {
          try { setContent(JSON.parse(rb.content)); } catch { setContent(null); }
        } else {
          setContent(null);
        }
      }).catch(() => setContent(null));
    });
  }, [refId]);

  if (!refId) return null;
  if (!content) {
    return (
      <section className="max-w-3xl mx-auto px-6 py-4">
        <div className="rounded border border-dashed p-4 text-center text-sm text-muted-foreground">
          Reusable Block: {refId}
        </div>
      </section>
    );
  }

  const blocks = Array.isArray(content) ? content : [content];
  return <BlockRenderer blocks={blocks} />;
}
