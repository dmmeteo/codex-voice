class Element {
  constructor(attributes = {}) {
    Object.assign(this, {value: '', textContent: '', hidden: false, disabled: false, checked: false, dataset: {}, children: [], listeners: {}, parent: null}, attributes);
  }
  get textContent() { return this.children.length ? this.children.map(child => child.textContent).join('') : this.text; }
  set textContent(value) { this.children = []; this.text = String(value); }
  set innerHTML(_) { throw new Error('innerHTML is not allowed'); }
  addEventListener(type, listener) { this.listeners[type] = listener; }
  adopt(children) { for (const child of children) { child.remove(); child.parent = this; } return children; }
  replaceChildren(...children) { for (const child of this.children) child.parent = null; this.children = this.adopt(children); }
  append(...children) { this.children.push(...this.adopt(children)); }
  prepend(...children) { this.children.unshift(...this.adopt(children)); }
  remove() { const index = this.parent?.children.indexOf(this) ?? -1; if (index >= 0) this.parent.children.splice(index, 1); this.parent = null; }
  get lastChild() { return this.children.at(-1) ?? null; }
}

/** Elements with an id, carrying the attributes the controllers read. */
export function fakeDocument(html) {
  const elements = new Map();
  for (const [tag, name, id] of html.matchAll(/<(\w+)[^>]*\sid="([^"]+)"[^>]*>/g)) {
    const selected = name === 'select' ? html.slice(html.indexOf(tag)).match(/<option value="([^"]+)" selected>/)?.[1] : undefined;
    elements.set(id, new Element({
      tagName: name.toUpperCase(),
      checked: /\schecked(?:\s|>)/.test(tag), disabled: /\sdisabled(?:\s|>)/.test(tag), hidden: /\shidden(?:\s|>)/.test(tag),
      value: selected ?? '', files: name === 'input' && /type="file"/.test(tag) ? [] : undefined,
    }));
  }
  return {getElementById: id => elements.get(id), createElement: tag => new Element({tagName: tag.toUpperCase()})};
}
