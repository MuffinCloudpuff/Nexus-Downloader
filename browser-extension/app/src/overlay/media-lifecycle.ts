const HOST_ID = "ghostDownloaderMediaDownload";
const OWNER_ATTRIBUTE = "data-nexus-media-overlay-owner";
const GUARD_ID = "nexusMediaOverlayGuard";
const CLAIM_EVENT = "nexus-media-overlay-claimed";

/** Own a document's overlay across extension isolated-world replacements. */
export function claimMediaOverlay(host: HTMLElement, onRelease: () => void, contextValid: () => boolean) {
  const doc = host.ownerDocument;
  const owner = crypto.randomUUID();
  let released = false;
  const guard = doc.createElement("style");
  guard.id = GUARD_ID;
  // Legacy content scripts can keep reattaching their detached host on a timer.
  // Hide those synchronously, then remove them in the mutation microtask.
  guard.textContent = `#${HOST_ID}:not([${OWNER_ATTRIBUTE}="${owner}"]) { display: none !important; }`;
  host.id = HOST_ID;
  host.setAttribute(OWNER_ATTRIBUTE, owner);

  function ownsDocument() {
    return doc.documentElement?.getAttribute(OWNER_ATTRIBUTE) === owner;
  }

  function release() {
    if (released) return;
    released = true;
    observer.disconnect();
    doc.removeEventListener(CLAIM_EVENT, checkOwner);
    host.remove();
    if (ownsDocument()) {
      doc.documentElement.removeAttribute(OWNER_ATTRIBUTE);
      // Keep a tombstone until the next owner arrives: legacy timers cannot be
      // canceled from another isolated world, even after this context expires.
      guard.textContent = `#${HOST_ID} { display: none !important; }`;
    }
    onRelease();
  }

  function active() {
    let valid = false;
    try { valid = contextValid(); } catch { /* Extension context was invalidated. */ }
    if (!released && (!ownsDocument() || !valid)) release();
    return !released;
  }

  function checkOwner() { active(); }

  function removeForeignHosts(node: Node) {
    if (!(node instanceof Element)) return;
    if (node.id === HOST_ID && node !== host) node.remove();
    for (const candidate of Array.from(node.querySelectorAll(`#${HOST_ID}`))) {
      if (candidate !== host) candidate.remove();
    }
  }

  const observer = new MutationObserver(records => {
    if (!active()) return;
    for (const record of records) {
      for (const node of Array.from(record.addedNodes)) removeForeignHosts(node);
    }
  });

  doc.documentElement.setAttribute(OWNER_ATTRIBUTE, owner);
  doc.dispatchEvent(new Event(CLAIM_EVENT));
  doc.getElementById(GUARD_ID)?.remove();
  doc.documentElement.appendChild(guard);
  removeForeignHosts(doc.documentElement);
  doc.addEventListener(CLAIM_EVENT, checkOwner);
  observer.observe(doc, {childList: true, subtree: true});
  return {active, release};
}
