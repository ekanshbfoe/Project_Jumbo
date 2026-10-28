/**
 * Unsaved work on a screen (a pasted Gemini answer, edited narration), kept in this browser so leaving the screen
 * or the app does not lose it. Storage can be missing or full; then the work simply lives as long as the screen.
 */
const key = (project: string, chapter: string, what: string) => `jumbo:draft:${project}:${chapter}:${what}`;

export function loadDraft<T>(project: string, chapter: string, what: string): T | undefined {
  try {
    const text = localStorage.getItem(key(project, chapter, what));
    return text ? (JSON.parse(text) as T) : undefined;
  } catch {
    return undefined;
  }
}

export function saveDraft(project: string, chapter: string, what: string, value: unknown) {
  try {
    localStorage.setItem(key(project, chapter, what), JSON.stringify(value));
  } catch {
    // Not kept; the screen still holds it.
  }
}

export function clearDraft(project: string, chapter: string, what: string) {
  try {
    localStorage.removeItem(key(project, chapter, what));
  } catch {
    // Nothing to clear.
  }
}
