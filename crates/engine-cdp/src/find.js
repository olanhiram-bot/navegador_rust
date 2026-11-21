function (query, index) {
  const doc = document;
  const ALL = 'navegador-busqueda';
  const CURRENT = 'navegador-busqueda-actual';

  const clear = () => {
    if (window.CSS && CSS.highlights) {
      CSS.highlights.delete(ALL);
      CSS.highlights.delete(CURRENT);
    }
  };

  if (!query) {
    clear();
    return JSON.stringify({ total: 0, current: -1 });
  }

  // El estilo de los resaltados va una sola vez; no modifica el contenido.
  if (!doc.getElementById('navegador-busqueda-estilo')) {
    const style = doc.createElement('style');
    style.id = 'navegador-busqueda-estilo';
    style.textContent =
      '::highlight(' + ALL + '){background-color:rgba(255,214,10,.35);}' +
      '::highlight(' + CURRENT + '){background-color:rgba(255,150,40,.7);color:inherit;}';
    (doc.head || doc.documentElement).appendChild(style);
  }

  const needle = query.toLocaleLowerCase();
  const ranges = [];
  const MAX = 500;
  const walker = doc.createTreeWalker(doc.body || doc.documentElement, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      const tag = node.parentElement ? node.parentElement.tagName : '';
      if (['SCRIPT', 'STYLE', 'NOSCRIPT'].includes(tag)) return NodeFilter.FILTER_REJECT;
      return node.nodeValue && node.nodeValue.trim()
        ? NodeFilter.FILTER_ACCEPT
        : NodeFilter.FILTER_REJECT;
    },
  });
  while (walker.nextNode() && ranges.length < MAX) {
    const text = walker.currentNode.nodeValue.toLocaleLowerCase();
    let at = text.indexOf(needle);
    while (at !== -1 && ranges.length < MAX) {
      const range = doc.createRange();
      range.setStart(walker.currentNode, at);
      range.setEnd(walker.currentNode, at + needle.length);
      ranges.push(range);
      at = text.indexOf(needle, at + needle.length);
    }
  }

  const total = ranges.length;
  if (!total || !(window.CSS && CSS.highlights)) {
    clear();
    return JSON.stringify({ total, current: -1 });
  }

  const current = ((index % total) + total) % total;
  CSS.highlights.set(ALL, new Highlight(...ranges));
  CSS.highlights.set(CURRENT, new Highlight(ranges[current]));
  const node = ranges[current].startContainer.parentElement;
  if (node) node.scrollIntoView({ block: 'center', behavior: 'instant' });
  return JSON.stringify({ total, current });
}
