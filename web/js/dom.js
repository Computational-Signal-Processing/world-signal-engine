/* Small DOM helpers.
 *
 * Blocks build their own trees, so the two constructors they all reach for live
 * here rather than being re-implemented per block. */

/** Create an HTML element. `props` sets attributes; `text` sets text content. */
export function el(tag, props = {}, children = []) {
  const node = document.createElement(tag);
  applyProps(node, props);
  for (const child of children) {
    if (child == null || child === false) continue;
    node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
}

/** Create an SVG element. SVG needs the namespaced constructor. */
export function svg(tag, props = {}, children = []) {
  const node = document.createElementNS("http://www.w3.org/2000/svg", tag);
  applyProps(node, props, true);
  for (const child of children) {
    if (child == null || child === false) continue;
    node.append(child);
  }
  return node;
}

const PROPS_AS_PROPERTIES = new Set(["value", "checked", "disabled", "textContent", "hidden", "selected"]);

function applyProps(node, props, isSvg = false) {
  for (const [key, value] of Object.entries(props)) {
    if (value == null || value === false) continue;
    if (key === "class" || key === "className") {
      node.setAttribute("class", String(value));
    } else if (key === "text") {
      node.textContent = String(value);
    } else if (key === "dataset") {
      for (const [k, v] of Object.entries(value)) {
        if (v != null) node.dataset[k] = String(v);
      }
    } else if (key === "style" && typeof value === "object") {
      for (const [k, v] of Object.entries(value)) node.style.setProperty(k, String(v));
    } else if (key === "on" && typeof value === "object") {
      for (const [k, fn] of Object.entries(value)) node.addEventListener(k, fn);
    } else if (!isSvg && PROPS_AS_PROPERTIES.has(key)) {
      node[key] = value;
    } else {
      node.setAttribute(key, value === true ? "" : String(value));
    }
  }
}

/** Remove every child of `node` without touching listeners held elsewhere. */
export function clear(node) {
  while (node.firstChild) node.removeChild(node.firstChild);
  return node;
}

/** Replace `node`'s children with `children` in one pass. */
export function replace(node, children) {
  clear(node);
  for (const child of children) {
    if (child == null || child === false) continue;
    node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
}

/**
 * A cleanup bag. Blocks register every timer, observer and listener they open
 * and hand the bag back on unmount, so nothing outlives the region that opened
 * it. A screen meant to stay up for weeks cannot leak a timer per update.
 */
export function cleanupBag() {
  const tasks = [];
  return {
    add(fn) { if (typeof fn === "function") tasks.push(fn); return fn; },
    /** Track an interval and return its handle. */
    interval(fn, ms) {
      const id = setInterval(fn, ms);
      tasks.push(() => clearInterval(id));
      return id;
    },
    /** Track a timeout and return its handle. */
    timeout(fn, ms) {
      const id = setTimeout(fn, ms);
      tasks.push(() => clearTimeout(id));
      return id;
    },
    /** Track a frame loop; the callback receives the frame time. */
    frame(fn) {
      let handle = 0;
      let stopped = false;
      const step = (now) => {
        if (stopped) return;
        fn(now);
        handle = requestAnimationFrame(step);
      };
      handle = requestAnimationFrame(step);
      tasks.push(() => { stopped = true; cancelAnimationFrame(handle); });
      return () => { stopped = true; cancelAnimationFrame(handle); };
    },
    /** Track a listener on any target. */
    listen(target, type, fn, opts) {
      target.addEventListener(type, fn, opts);
      tasks.push(() => target.removeEventListener(type, fn, opts));
      return fn;
    },
    /** Track an observer. */
    observe(observer) {
      tasks.push(() => observer.disconnect());
      return observer;
    },
    dispose() {
      while (tasks.length) {
        const fn = tasks.pop();
        try { fn(); } catch (_) { /* one bad cleanup must not strand the rest */ }
      }
    },
  };
}
