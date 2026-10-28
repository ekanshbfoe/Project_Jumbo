import { useState } from "react";

/** The kinds of content Jumbo can turn into videos. Only manga and manhwa exist so far. */
const CONTENT = [
  { id: "manga", title: "Manga / Manhwa", subtitle: "Chapters as PDFs, retold as narrated motion recaps.", ready: true },
  { id: "book", title: "Book Explanation", subtitle: "Non-fiction books explained chapter by chapter.", ready: false },
  { id: "novel", title: "Novels", subtitle: "Web novels and light novels retold as audio-visual recaps.", ready: false },
] as const;

/** First screen: pick what to make. Content types that are not built yet shake and say so. */
export function HomeScreen({ onManga }: { onManga: () => void }) {
  const [shaking, setShaking] = useState<string>();

  return (
    <div className="welcome stack">
      <div>
        <h1 style={{ marginBottom: 4 }}>Jumbo</h1>
        <p className="muted" style={{ marginTop: 0 }}>
          What are we making today? Projects live entirely in a folder on this PC.
        </p>
      </div>
      <div className="content-cards">
        {CONTENT.map((c) => (
          <button
            key={c.id}
            className={`content-card ${c.ready ? "" : "soon"} ${shaking === c.id ? "shake" : ""}`}
            onClick={() => (c.ready ? onManga() : setShaking(c.id))}
            onAnimationEnd={() => setShaking(undefined)}
          >
            <strong>{c.title}</strong>
            <span className="muted">{c.subtitle}</span>
            {c.ready ? null : <span className="badge">Coming soon</span>}
          </button>
        ))}
      </div>
    </div>
  );
}
