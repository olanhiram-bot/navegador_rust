function (x, y) {
  const el = document.elementFromPoint(x, y);
  if (!el) return JSON.stringify({ selector: null });

  let target = el.closest(
    'a[href], button, input, select, textarea, summary, [role=button], ' +
    '[role=link], [onclick], [contenteditable]:not([contenteditable=false]), video, label'
  );
  if (target?.tagName === 'LABEL') target = target.control;
  if (!target) return JSON.stringify({ selector: null });

  // Mismo cssPath que snapshot.js: la ref solo vale si el selector coincide
  // exactamente con el del snapshot.
  const cssPath = (el) => {
    const uniqueId = (node) => {
      if (!node.id) return null;
      try {
        const matches = document.querySelectorAll('#' + CSS.escape(node.id));
        return matches.length === 1 ? '#' + CSS.escape(node.id) : null;
      } catch (e) {
        return null;
      }
    };

    const direct = uniqueId(el);
    if (direct) return direct;

    const parts = [];
    let node = el;
    while (node && node.nodeType === 1 && node !== document.documentElement) {
      const id = uniqueId(node);
      if (id) {
        parts.unshift(id);
        break;
      }
      const tag = node.tagName.toLowerCase();
      const parent = node.parentElement;
      if (!parent) {
        parts.unshift(tag);
        break;
      }
      const sameTag = Array.from(parent.children).filter(
        (child) => child.tagName === node.tagName
      );
      parts.unshift(
        sameTag.length === 1
          ? tag
          : tag + ':nth-of-type(' + (sameTag.indexOf(node) + 1) + ')'
      );
      node = parent;
    }
    return parts.join(' > ');
  };

  return JSON.stringify({ selector: cssPath(target) });
}
