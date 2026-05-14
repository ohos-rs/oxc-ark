// Portions of this file are derived from Oxc's oxlint implementation.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

var y = Object.defineProperty
var R = Object.getOwnPropertyDescriptor
var J = Object.getOwnPropertyNames
var L = Object.prototype.hasOwnProperty
var M = (t, e) => {
    for (var n in e) y(t, n, { get: e[n], enumerable: !0 })
  },
  T = (t, e, n, o) => {
    if ((e && typeof e == 'object') || typeof e == 'function')
      for (let r of J(e))
        !L.call(t, r) && r !== n && y(t, r, { get: () => e[r], enumerable: !(o = R(e, r)) || o.enumerable })
    return t
  }
var $ = (t) => T(y({}, '__esModule', { value: !0 }), t)
var K = {}
M(K, { loadJsConfigs: () => S, loadVitePlusConfigs: () => k })
module.exports = $(K)
function g(t) {
  try {
    if (t instanceof Error) {
      let { stack: n } = t
      if (typeof n == 'string' && n !== '') return n
    }
    let { message: e } = t
    if (typeof e == 'string' && e !== '') return e
  } catch {}
  return 'Unknown error'
}
var {
    prototype: W,
    hasOwn: z,
    keys: G,
    values: X,
    freeze: q,
    preventExtensions: Y,
    defineProperty: B,
    defineProperties: Q,
    create: Z,
    assign: tt,
    getPrototypeOf: et,
    setPrototypeOf: nt,
    entries: rt,
  } = Object,
  { prototype: ot, isArray: st, from: it } = Array,
  { min: at, max: ct, floor: ft } = Math,
  { parse: ut, stringify: p } = JSON,
  { ownKeys: lt } = Reflect,
  { iterator: gt } = Symbol,
  { fromCodePoint: pt } = String,
  { now: b } = Date
var h = require('node:path'),
  x = require('node:url')
var A = '^20.19.0 || >=22.18.0',
  F = new Set(['.ts', '.mts', '.cts'])
function v(t) {
  if (!t.startsWith('file:')) return t
  try {
    return (0, x.fileURLToPath)(t)
  } catch {
    return t
  }
}
function U(t) {
  let e = (0, h.extname)(v(t)).toLowerCase()
  return F.has(e)
}
function _(t) {
  if (t?.code === 'ERR_UNKNOWN_FILE_EXTENSION') return !0
  let e = t?.message
  return typeof e == 'string' && /unknown(?: or unsupported)? file extension/i.test(e)
}
function O(t, e, n = process.version) {
  return !U(e) || !_(t)
    ? null
    : `${g(t)}

TypeScript config files require Node.js ${A}.
Detected Node.js ${n}.
Please upgrade Node.js or use a JSON config file instead.`
}
var d = (t) => typeof t == 'object' && t !== null && !Array.isArray(t)
function C(t) {
  let e = new WeakSet(),
    n = new WeakSet(),
    o = [],
    r = [],
    u = (s, i, f) => {
      let a = f === -1 ? `${i} -> ${i}` : [...r.slice(f), i].join(' -> ')
      return `\`extends\` contains a circular reference.

${s} points back to ${i}
Cycle: ${a}`
    },
    c = (s, i) => {
      if (e.has(s)) return
      if (n.has(s)) {
        let a = o.indexOf(s),
          l = a === -1 ? '<unknown>' : r[a]
        throw new Error(u(i, l, a))
      }
      ;(n.add(s), o.push(s), r.push(i))
      let f = s.extends
      if (f !== void 0) {
        if (!Array.isArray(f))
          throw new Error('`extends` must be an array of config objects (strings/paths are not supported).')
        for (let a = 0; a < f.length; a++) {
          let l = f[a]
          if (!d(l)) throw new Error(`\`extends[${a}]\` must be a config object (strings/paths are not supported).`)
          let w = `${i}.extends[${a}]`
          if (n.has(l)) {
            let m = o.indexOf(l),
              N = m === -1 ? '<unknown>' : r[m]
            throw new Error(u(w, N, m))
          }
          c(l, w)
        }
      }
      ;(n.delete(s), o.pop(), r.pop(), e.add(s))
    }
  c(t, '<root>')
}
async function P(t, e) {
  let r = (await import(new URL(`file://${t}?cache=${e}`).href)).default
  if (r === void 0) throw new Error('Configuration file has no default export.')
  return r
}
async function I(t, e) {
  let n = await P(t, e)
  if (!d(n)) throw new Error('Configuration file must have a default export that is an object.')
  return (C(n), { path: t, config: n })
}
var E = 'lint'
async function D(t, e) {
  let n = await P(t, e)
  if (!d(n)) return { path: t, config: null }
  let o = n[E]
  if (o === void 0) return { path: t, config: null }
  if (!d(o)) throw new Error(`The \`${E}\` field in the default export must be an object.`)
  return (C(o), { path: t, config: o })
}
async function j(t, e) {
  try {
    let n = b(),
      o = await Promise.allSettled(t.map((c) => e(c, n))),
      r = [],
      u = []
    for (let c = 0; c < o.length; c++) {
      let s = o[c]
      if (s.status === 'fulfilled') r.push(s.value)
      else {
        let i = t[c],
          f = O(s.reason, i)
        u.push({ path: i, error: f ?? g(s.reason) })
      }
    }
    return u.length > 0 ? p({ Failures: u }) : p({ Success: r })
  } catch (n) {
    return p({ Error: g(n) })
  }
}
var S = (t) => j(t, I),
  k = (t) => j(t, D)
0 && (module.exports = { loadJsConfigs, loadVitePlusConfigs })
