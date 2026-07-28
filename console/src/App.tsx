import { useEffect, useState } from "react";
import { useSnapshot } from "./lib/useSnapshot";
import { PageTabs, TopBar, type PageId } from "./shell/Shell";
import { MediaPage } from "./pages/MediaPage";
import { ScreensPage } from "./pages/ScreensPage";
import { ChannelsPage } from "./pages/ChannelsPage";
import { CuesPage } from "./pages/CuesPage";
import { TemplatesPage } from "./pages/TemplatesPage";
import { GridPage } from "./pages/GridPage";

const PAGE_IDS: PageId[] = ["media", "screens", "channels", "cues", "templates", "grid"];

/**
 * A page requested via `?window=<page>` — a detached, single-page view for a
 * second display. Read once at mount: a window opened to show one thing is not
 * expected to navigate.
 */
function standaloneWindowPage(): PageId | null {
  const id = new URLSearchParams(window.location.search).get("window");
  return (PAGE_IDS as string[]).includes(id ?? "") ? (id as PageId) : null;
}

/** URL that opens `page` alone in a new window — the "detach" target. */
export function popOutUrl(page: PageId): string {
  const url = new URL(window.location.href);
  url.search = `?window=${page}`;
  return url.toString();
}

export function App() {
  const { snapshot, status, error } = useSnapshot();
  const [standalone] = useState(standaloneWindowPage);
  const [page, setPage] = useState<PageId>(() => standalone ?? "media");
  const [dismissed, setDismissed] = useState<string | null>(null);

  // The screen most actions target. Held at app level so switching pages does
  // not lose the operator's place — picking a clip on Media and firing it is a
  // two-page move.
  const [target, setTarget] = useState<string | null>(null);
  const screens = snapshot.show.screens;

  // Keep the target valid as screens come and go.
  useEffect(() => {
    if (screens.length === 0) {
      if (target !== null) setTarget(null);
    } else if (!screens.some((s) => s.id === target)) {
      setTarget(screens[0].id);
    }
  }, [screens, target]);

  const banner = error && error !== dismissed ? error : null;
  const warnings = snapshot.warnings;

  const pageProps = { snapshot, target, setTarget };

  return (
    <div className="app">
      <TopBar status={status} snapshot={snapshot} />
      {banner && (
        <div className="banner error">
          <span>{banner}</span>
          <button className="banner-close" onClick={() => setDismissed(error)}>
            ✕
          </button>
        </div>
      )}
      {!banner &&
        warnings.map((w) => (
          <div className="banner" key={w}>
            <span>⚠ {w}</span>
          </div>
        ))}
      <main className="app-body">
        {page === "media" && <MediaPage {...pageProps} />}
        {page === "screens" && <ScreensPage {...pageProps} />}
        {page === "channels" && <ChannelsPage {...pageProps} />}
        {page === "cues" && <CuesPage {...pageProps} />}
        {page === "templates" && <TemplatesPage {...pageProps} />}
        {page === "grid" && <GridPage {...pageProps} />}
      </main>
      {/* A detached window shows one page and nothing to switch away from —
          that is the point of putting it on a second display. */}
      {!standalone && <PageTabs active={page} onSelect={setPage} />}
    </div>
  );
}
