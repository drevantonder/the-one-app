import { c as e } from "react/compiler-runtime";
import * as t from "react";
import { createContext as n, useState as r } from "react";
import { createRoot as i } from "react-dom/client";
import { AccountList as a, AlertDialog as o, AlertDialogAction as s, AlertDialogCancel as c, AlertDialogContent as l, AlertDialogDescription as u, AlertDialogFooter as d, AlertDialogHeader as f, AlertDialogTitle as p, AlertDialogTrigger as m, AppIcon as h, Avatar as g, AvatarFallback as _, AvatarImage as v, Badge as y, Button as b, Card as x, CardContent as S, CardDescription as ee, CardFooter as te, CardHeader as ne, CardTitle as re, CodeView as ie, CopyButton as ae, Dialog as oe, DialogContent as se, DialogDescription as ce, DialogFooter as le, DialogHeader as ue, DialogTitle as de, DialogTrigger as fe, DiffView as pe, DropdownMenu as me, DropdownMenuContent as he, DropdownMenuItem as ge, DropdownMenuLabel as _e, DropdownMenuSeparator as ve, DropdownMenuTrigger as ye, Empty as be, Grid as xe, Hint as C, Icon as Se, Input as Ce, List as we, ListNote as w, ListRow as T, ListSkeleton as E, LoadFailure as D, Loading as O, LoadingButton as k, Problem as Te, Progress as Ee, ResponsiveDialog as De, Select as Oe, SelectContent as ke, SelectItem as A, SelectTrigger as Ae, SelectValue as je, Separator as Me, Sheet as Ne, SheetContent as Pe, SheetDescription as Fe, SheetHeader as Ie, SheetTitle as Le, SheetTrigger as Re, Shell as ze, Skeleton as Be, Sparkline as Ve, Split as He, Switch as j, Tabs as Ue, TabsContent as We, TabsList as Ge, TabsTrigger as Ke, showAlert as M, showMenu as N, showSheet as P, useDialog as F } from "@one/ui";
import { Fragment as I, jsx as L, jsxs as R } from "react/jsx-runtime";
// node_modules/@phosphor-icons/react/dist/defs/DotsThree.es.js
var z = /* @__PURE__ */ new Map([
	["bold", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M144,128a16,16,0,1,1-16-16A16,16,0,0,1,144,128ZM60,112a16,16,0,1,0,16,16A16,16,0,0,0,60,112Zm136,0a16,16,0,1,0,16,16A16,16,0,0,0,196,112Z" }))],
	["duotone", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", {
		d: "M240,96v64a16,16,0,0,1-16,16H32a16,16,0,0,1-16-16V96A16,16,0,0,1,32,80H224A16,16,0,0,1,240,96Z",
		opacity: "0.2"
	}), /* @__PURE__ */ t.createElement("path", { d: "M140,128a12,12,0,1,1-12-12A12,12,0,0,1,140,128Zm56-12a12,12,0,1,0,12,12A12,12,0,0,0,196,116ZM60,116a12,12,0,1,0,12,12A12,12,0,0,0,60,116Z" }))],
	["fill", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M224,80H32A16,16,0,0,0,16,96v64a16,16,0,0,0,16,16H224a16,16,0,0,0,16-16V96A16,16,0,0,0,224,80ZM60,140a12,12,0,1,1,12-12A12,12,0,0,1,60,140Zm68,0a12,12,0,1,1,12-12A12,12,0,0,1,128,140Zm68,0a12,12,0,1,1,12-12A12,12,0,0,1,196,140Z" }))],
	["light", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M138,128a10,10,0,1,1-10-10A10,10,0,0,1,138,128ZM60,118a10,10,0,1,0,10,10A10,10,0,0,0,60,118Zm136,0a10,10,0,1,0,10,10A10,10,0,0,0,196,118Z" }))],
	["regular", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M140,128a12,12,0,1,1-12-12A12,12,0,0,1,140,128Zm56-12a12,12,0,1,0,12,12A12,12,0,0,0,196,116ZM60,116a12,12,0,1,0,12,12A12,12,0,0,0,60,116Z" }))],
	["thin", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M136,128a8,8,0,1,1-8-8A8,8,0,0,1,136,128Zm-76-8a8,8,0,1,0,8,8A8,8,0,0,0,60,120Zm136,0a8,8,0,1,0,8,8A8,8,0,0,0,196,120Z" }))]
]), B = /* @__PURE__ */ new Map([
	["bold", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M232.49,215.51,185,168a92.12,92.12,0,1,0-17,17l47.53,47.54a12,12,0,0,0,17-17ZM44,112a68,68,0,1,1,68,68A68.07,68.07,0,0,1,44,112Z" }))],
	["duotone", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", {
		d: "M192,112a80,80,0,1,1-80-80A80,80,0,0,1,192,112Z",
		opacity: "0.2"
	}), /* @__PURE__ */ t.createElement("path", { d: "M229.66,218.34,179.6,168.28a88.21,88.21,0,1,0-11.32,11.31l50.06,50.07a8,8,0,0,0,11.32-11.32ZM40,112a72,72,0,1,1,72,72A72.08,72.08,0,0,1,40,112Z" }))],
	["fill", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M168,112a56,56,0,1,1-56-56A56,56,0,0,1,168,112Zm61.66,117.66a8,8,0,0,1-11.32,0l-50.06-50.07a88,88,0,1,1,11.32-11.31l50.06,50.06A8,8,0,0,1,229.66,229.66ZM112,184a72,72,0,1,0-72-72A72.08,72.08,0,0,0,112,184Z" }))],
	["light", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M228.24,219.76l-51.38-51.38a86.15,86.15,0,1,0-8.48,8.48l51.38,51.38a6,6,0,0,0,8.48-8.48ZM38,112a74,74,0,1,1,74,74A74.09,74.09,0,0,1,38,112Z" }))],
	["regular", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M229.66,218.34l-50.07-50.06a88.11,88.11,0,1,0-11.31,11.31l50.06,50.07a8,8,0,0,0,11.32-11.32ZM40,112a72,72,0,1,1,72,72A72.08,72.08,0,0,1,40,112Z" }))],
	["thin", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M226.83,221.17l-52.7-52.7a84.1,84.1,0,1,0-5.66,5.66l52.7,52.7a4,4,0,0,0,5.66-5.66ZM36,112a76,76,0,1,1,76,76A76.08,76.08,0,0,1,36,112Z" }))]
]), V = /* @__PURE__ */ new Map([
	["bold", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M228,128a12,12,0,0,1-12,12H140v76a12,12,0,0,1-24,0V140H40a12,12,0,0,1,0-24h76V40a12,12,0,0,1,24,0v76h76A12,12,0,0,1,228,128Z" }))],
	["duotone", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", {
		d: "M216,56V200a16,16,0,0,1-16,16H56a16,16,0,0,1-16-16V56A16,16,0,0,1,56,40H200A16,16,0,0,1,216,56Z",
		opacity: "0.2"
	}), /* @__PURE__ */ t.createElement("path", { d: "M224,128a8,8,0,0,1-8,8H136v80a8,8,0,0,1-16,0V136H40a8,8,0,0,1,0-16h80V40a8,8,0,0,1,16,0v80h80A8,8,0,0,1,224,128Z" }))],
	["fill", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M208,32H48A16,16,0,0,0,32,48V208a16,16,0,0,0,16,16H208a16,16,0,0,0,16-16V48A16,16,0,0,0,208,32ZM184,136H136v48a8,8,0,0,1-16,0V136H72a8,8,0,0,1,0-16h48V72a8,8,0,0,1,16,0v48h48a8,8,0,0,1,0,16Z" }))],
	["light", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M222,128a6,6,0,0,1-6,6H134v82a6,6,0,0,1-12,0V134H40a6,6,0,0,1,0-12h82V40a6,6,0,0,1,12,0v82h82A6,6,0,0,1,222,128Z" }))],
	["regular", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M224,128a8,8,0,0,1-8,8H136v80a8,8,0,0,1-16,0V136H40a8,8,0,0,1,0-16h80V40a8,8,0,0,1,16,0v80h80A8,8,0,0,1,224,128Z" }))],
	["thin", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M220,128a4,4,0,0,1-4,4H132v84a4,4,0,0,1-8,0V132H40a4,4,0,0,1,0-8h84V40a4,4,0,0,1,8,0v84h84A4,4,0,0,1,220,128Z" }))]
]), H = n({
	color: "currentColor",
	size: "1em",
	weight: "regular",
	mirrored: !1
}), U = t.forwardRef((e, n) => {
	let { alt: r, color: i, size: a, weight: o, mirrored: s, children: c, weights: l, ...u } = e, { color: d = "currentColor", size: f, weight: p = "regular", mirrored: m = !1, ...h } = t.useContext(H);
	return /* @__PURE__ */ t.createElement("svg", {
		ref: n,
		xmlns: "http://www.w3.org/2000/svg",
		width: a ?? f,
		height: a ?? f,
		fill: i ?? d,
		viewBox: "0 0 256 256",
		transform: s || m ? "scale(-1, 1)" : void 0,
		...h,
		...u
	}, !!r && /* @__PURE__ */ t.createElement("title", null, r), c, l.get(o ?? p));
});
U.displayName = "IconBase";
// node_modules/@phosphor-icons/react/dist/csr/DotsThree.es.js
var qe = t.forwardRef((e, n) => /* @__PURE__ */ t.createElement(U, {
	ref: n,
	...e,
	weights: z
}));
qe.displayName = "DotsThreeIcon";
// node_modules/@phosphor-icons/react/dist/csr/MagnifyingGlass.es.js
var Je = t.forwardRef((e, n) => /* @__PURE__ */ t.createElement(U, {
	ref: n,
	...e,
	weights: B
}));
Je.displayName = "MagnifyingGlassIcon";
// node_modules/@phosphor-icons/react/dist/csr/Plus.es.js
var Ye = t.forwardRef((e, n) => /* @__PURE__ */ t.createElement(U, {
	ref: n,
	...e,
	weights: V
}));
Ye.displayName = "PlusIcon";
// gallery/gallery.tsx
var W = [
	{
		name: "agent-devices",
		title: "Agent Devices",
		icon: "browsers",
		color: "#a2845e"
	},
	{
		name: "board",
		title: "Agent Board",
		icon: "chat-text",
		color: "#ff9500"
	},
	{
		name: "device-activity",
		title: "Device Activity",
		icon: "monitor",
		color: "#30b0c7"
	},
	{
		name: "repos",
		title: "Repos",
		icon: "git-branch",
		color: "#5856d6"
	},
	{
		name: "computer",
		title: "Computer Use",
		icon: "cursor-click",
		color: "#5e5ce6"
	},
	{
		name: "groups",
		title: "Groups",
		icon: "users-three",
		color: "#34c759"
	},
	{
		name: "connect-ai",
		title: "Connect Your AI",
		icon: "plugs-connected",
		color: "#ff2d55"
	},
	{
		name: "backups",
		title: "Backups",
		icon: "hard-drives",
		color: "#ff9500"
	},
	{
		name: "labs",
		title: "Labs",
		icon: "flask",
		color: "#af52de"
	},
	{
		name: "devices",
		title: "Devices",
		icon: "devices",
		color: "#8e8e93"
	}
], Xe = "// Keeps the last ten answers, newest first.\nimport { createHash } from \"node:crypto\"\n\nconst KEPT = 10\nconst seen = new Map<string, number>()\n\n/** The answer for `key`, or nothing if it's been dropped. */\nexport function recall(key: string, now = Date.now()): string | undefined {\n  const at = seen.get(key)\n  if (at === undefined || now - at > 60_000) return undefined\n  return createHash(\"sha256\").update(key).digest(\"hex\").slice(0, 8)\n}\n\nexport function keep(key: string) {\n  seen.set(key, Date.now())\n  for (const [old] of [...seen].slice(0, -KEPT)) seen.delete(old)\n}\n", Ze = [{
	a: 8,
	b: 8,
	lines: [
		" export function recall(key: string, now = Date.now()): string | undefined {",
		"-  const at = seen.get(key)",
		"-  if (at === undefined) return undefined",
		"+  const at = seen.get(key)",
		"+  if (at === undefined || now - at > 60_000) return undefined",
		"   return createHash(\"sha256\").update(key).digest(\"hex\").slice(0, 8)",
		" }",
		" ",
		"+export function keep(key: string) {",
		"+  seen.set(key, Date.now())",
		"+  for (const [old] of [...seen].slice(0, -KEPT)) seen.delete(old)",
		"+}"
	]
}, {
	a: 40,
	b: 44,
	lines: [
		" const long = \"a line that is long enough that it has to wrap in the narrow one: the quick brown fox jumps over the lazy dog\"",
		"-const removed = 1",
		" const kept = 2"
	]
}], G = [
	{
		id: "anthropic",
		title: "Anthropic",
		sub: "Key · Used by AI and Models",
		app: {
			name: "ai",
			icon: "sparkle",
			color: "#d97757"
		}
	},
	{
		id: "github",
		title: "GitHub",
		sub: "Account · Used by Repos",
		app: {
			name: "github",
			icon: "github-logo",
			color: "#24292f"
		}
	},
	{
		id: "work",
		title: "andre@work",
		sub: "Google account · Used by Gmail and Drive",
		app: {
			name: "google",
			icon: "google-logo",
			color: "#4285f4"
		}
	},
	{
		id: "wifi",
		title: "Office Wi-Fi",
		sub: "network: Office-5G",
		app: {
			name: "wifi",
			icon: "wifi-high",
			color: "#8e8e93"
		}
	}
], K = [
	{
		account: "ana@example.org",
		ok: !0
	},
	{
		account: "work@example.org",
		ok: !0
	},
	{
		account: "old@example.org",
		ok: !1
	}
];
function Qe() {
	let t = e(21), [n, i] = r(K), [o, s] = r("personal"), c, l;
	t[0] === Symbol.for("react.memo_cache_sentinel") ? (c = (e) => i((t) => t.filter((t) => t.account !== e.account)), l = /* @__PURE__ */ L(k, {
		variant: "secondary",
		className: "self-start",
		run: J,
		children: "Add Account"
	}), t[0] = c, t[1] = l) : (c = t[0], l = t[1]);
	let u;
	t[2] === n ? u = t[3] : (u = /* @__PURE__ */ L(a, {
		title: "Accounts",
		accounts: n,
		line: X,
		signOut: c,
		children: l
	}), t[2] = n, t[3] = u);
	let d;
	t[4] === Symbol.for("react.memo_cache_sentinel") ? (d = /* @__PURE__ */ L("h2", {
		className: "text-sm font-medium text-muted-foreground",
		children: "Browser profile"
	}), t[4] = d) : d = t[4];
	let f;
	t[5] === Symbol.for("react.memo_cache_sentinel") ? (f = /* @__PURE__ */ L(Ae, {
		size: "sm",
		"aria-label": "Browser profile",
		children: /* @__PURE__ */ L(je, {})
	}), t[5] = f) : f = t[5];
	let p;
	t[6] === Symbol.for("react.memo_cache_sentinel") ? (p = /* @__PURE__ */ R(ke, {
		align: "end",
		position: "popper",
		children: [
			/* @__PURE__ */ L(A, {
				value: "personal",
				children: "personal"
			}),
			/* @__PURE__ */ L(A, {
				value: "work",
				children: "work"
			}),
			/* @__PURE__ */ L(A, {
				value: "new",
				children: "New profile…"
			})
		]
	}), t[6] = p) : p = t[6];
	let m;
	t[7] === o ? m = t[8] : (m = /* @__PURE__ */ L(T, {
		title: "Profile",
		trailing: /* @__PURE__ */ R(Oe, {
			value: o,
			onValueChange: s,
			children: [f, p]
		})
	}), t[7] = o, t[8] = m);
	let h;
	t[9] === Symbol.for("react.memo_cache_sentinel") ? (h = /* @__PURE__ */ L(w, { children: "A line of what to do, which wraps where a row's title would be cut." }), t[9] = h) : h = t[9];
	let g;
	t[10] === m ? g = t[11] : (g = /* @__PURE__ */ R(we, { children: [m, h] }), t[10] = m, t[11] = g);
	let _;
	t[12] === Symbol.for("react.memo_cache_sentinel") ? (_ = /* @__PURE__ */ L(C, { children: "A grey line under what it is about." }), t[12] = _) : _ = t[12];
	let v;
	t[13] === g ? v = t[14] : (v = /* @__PURE__ */ R("section", {
		className: "flex flex-col gap-2",
		children: [
			d,
			g,
			_
		]
	}), t[13] = g, t[14] = v);
	let y, b;
	t[15] === Symbol.for("react.memo_cache_sentinel") ? (y = /* @__PURE__ */ L(O, { children: "Signing In" }), b = /* @__PURE__ */ L(E, { rows: 2 }), t[15] = y, t[16] = b) : (y = t[15], b = t[16]);
	let x;
	t[17] === Symbol.for("react.memo_cache_sentinel") ? (x = /* @__PURE__ */ L(D, {
		problem: {
			title: "Couldn't load your accounts.",
			next: "Check your connection."
		},
		retry: q
	}), t[17] = x) : x = t[17];
	let S;
	return t[18] !== v || t[19] !== u ? (S = /* @__PURE__ */ R("div", {
		className: "flex flex-col gap-6",
		children: [
			u,
			v,
			y,
			b,
			x
		]
	}), t[18] = v, t[19] = u, t[20] = S) : S = t[20], S;
}
function q() {}
function J() {
	return new Promise(Y);
}
function Y(e) {
	return setTimeout(e, 1500);
}
function X(e, t) {
	return e.ok ? t ? null : { text: "Used first" } : {
		text: "Sign in again",
		warn: !0
	};
}
function $e() {
	let t = e(17), [n, r, i] = F(), a;
	t[0] === Symbol.for("react.memo_cache_sentinel") ? (a = /* @__PURE__ */ L(h, {
		app: W[0],
		size: 32
	}), t[0] = a) : a = t[0];
	let o;
	t[1] === r ? o = t[2] : (o = /* @__PURE__ */ L(T, {
		leading: a,
		title: "Agent Devices wants an Anthropic key",
		sub: "claude-code · Wants the Anthropic key to run the nightly checks on every repo, and to open pull requests for what it finds, until you take it back.",
		wrap: !0,
		trailing: /* @__PURE__ */ L(b, {
			variant: "secondary",
			size: "sm",
			onClick: () => r("Anthropic key"),
			children: "Choose"
		})
	}), t[1] = r, t[2] = o);
	let s;
	t[3] === Symbol.for("react.memo_cache_sentinel") ? (s = /* @__PURE__ */ L(g, {
		name: "Ann Lee",
		id: "g1",
		size: 32
	}), t[3] = s) : s = t[3];
	let c;
	t[4] === r ? c = t[5] : (c = /* @__PURE__ */ L(T, {
		leading: s,
		title: "Ann Lee, Bo Chen, Cy Dunn",
		sub: "Cut to one line, as a row's line is without wrap, however long it gets",
		trailing: /* @__PURE__ */ L(b, {
			variant: "secondary",
			size: "sm",
			onClick: () => r("Join"),
			children: "Join"
		})
	}), t[4] = r, t[5] = c);
	let l;
	t[6] !== o || t[7] !== c ? (l = /* @__PURE__ */ R(we, { children: [o, c] }), t[6] = o, t[7] = c, t[8] = l) : l = t[8];
	let u;
	t[9] !== n.for || t[10] !== n.n || t[11] !== n.open || t[12] !== i ? (u = n.n > 0 && /* @__PURE__ */ L(De, {
		open: n.open,
		onOpenChange: i,
		title: n.for ?? "",
		action: /* @__PURE__ */ L(b, {
			variant: "secondary",
			size: "sm",
			onClick: () => i(!1),
			children: "Cancel"
		}),
		children: /* @__PURE__ */ R("p", {
			className: "p-4 text-sm text-muted-foreground",
			children: [
				"Opened for ",
				n.for,
				"."
			]
		})
	}, n.n), t[9] = n.for, t[10] = n.n, t[11] = n.open, t[12] = i, t[13] = u) : u = t[13];
	let d;
	return t[14] !== l || t[15] !== u ? (d = /* @__PURE__ */ R(I, { children: [l, u] }), t[14] = l, t[15] = u, t[16] = d) : d = t[16], d;
}
function Z(t) {
	let n = e(5), { title: r, children: i } = t, a;
	n[0] === r ? a = n[1] : (a = /* @__PURE__ */ L("h2", {
		className: "text-sm font-medium text-muted-foreground",
		children: r
	}), n[0] = r, n[1] = a);
	let o;
	return n[2] !== i || n[3] !== a ? (o = /* @__PURE__ */ R("section", {
		className: "flex flex-col gap-3",
		children: [a, i]
	}), n[2] = i, n[3] = a, n[4] = o) : o = n[4], o;
}
function Q(t) {
	let n = e(2), { children: r } = t, i;
	return n[0] === r ? i = n[1] : (i = /* @__PURE__ */ L("div", {
		className: "flex flex-wrap items-center gap-2",
		children: r
	}), n[0] = r, n[1] = i), i;
}
function et() {
	let t = e(13), [n, i] = r(null), a;
	t[0] === n ? a = t[1] : (a = G.find((e) => e.id === n), t[0] = n, t[1] = a);
	let o = a, s = o?.title, c;
	t[2] === Symbol.for("react.memo_cache_sentinel") ? (c = () => i(null), t[2] = c) : c = t[2];
	let l;
	t[3] === n ? l = t[4] : (l = G.map((e) => /* @__PURE__ */ L(T, {
		leading: /* @__PURE__ */ L(h, {
			app: e.app,
			size: 32
		}),
		title: e.title,
		sub: e.sub,
		selected: e.id === n,
		onClick: () => i(e.id)
	}, e.id)), t[3] = n, t[4] = l);
	let u;
	t[5] === l ? u = t[6] : (u = /* @__PURE__ */ L(we, { children: l }), t[5] = l, t[6] = u);
	let d;
	t[7] === o ? d = t[8] : (d = o && /* @__PURE__ */ R(x, { children: [
		/* @__PURE__ */ R(ne, {
			className: "flex flex-row items-center gap-4",
			children: [
				/* @__PURE__ */ L(h, {
					app: o.app,
					size: 56
				}),
				/* @__PURE__ */ R("div", {
					className: "min-w-0 flex-1",
					children: [/* @__PURE__ */ L(re, {
						className: "text-lg",
						children: o.title
					}), /* @__PURE__ */ L(ee, { children: "You" })]
				}),
				/* @__PURE__ */ L(b, {
					variant: "outline",
					size: "sm",
					children: "Rename"
				})
			]
		}),
		/* @__PURE__ */ R(S, {
			className: "flex flex-col gap-3",
			children: [/* @__PURE__ */ R("div", {
				className: "flex items-center justify-between gap-3",
				children: [/* @__PURE__ */ L("span", {
					className: "text-sm",
					children: "AI"
				}), /* @__PURE__ */ L(j, { defaultChecked: !0 })]
			}), /* @__PURE__ */ R("div", {
				className: "flex items-center justify-between gap-3",
				children: [/* @__PURE__ */ L("span", {
					className: "text-sm",
					children: "AGENTS.md"
				}), /* @__PURE__ */ L(j, {})]
			})]
		}),
		/* @__PURE__ */ L(te, { children: /* @__PURE__ */ L(b, {
			variant: "destructive",
			size: "sm",
			children: "Delete Item"
		}) })
	] }), t[7] = o, t[8] = d);
	let f;
	return t[9] !== s || t[10] !== u || t[11] !== d ? (f = /* @__PURE__ */ L(He, {
		title: s,
		onClose: c,
		list: u,
		detail: d
	}), t[9] = s, t[10] = u, t[11] = d, t[12] = f) : f = t[12], f;
}
var $ = [
	{
		key: "user",
		title: "You",
		href: "#you",
		items: [...W.slice(0, 1).map((e) => ({
			key: e.name,
			title: e.title,
			href: `#${e.name}`,
			app: e,
			selected: !0
		})), {
			key: "accounts",
			title: "Accounts",
			href: "#accounts",
			folder: W.slice(4, 6)
		}]
	},
	{
		key: "global",
		title: "Everyone",
		href: "#everyone",
		items: W.slice(1, 4).map((e) => ({
			key: e.name,
			title: e.title,
			href: `#${e.name}`,
			app: e,
			badge: e.name === "board" ? 3 : 0
		}))
	},
	{
		key: "settings",
		title: "Settings",
		href: "#settings",
		items: [{
			key: "passkeys",
			title: "Passkeys",
			href: "https://example.com",
			external: !0,
			app: {
				name: "passkeys",
				icon: "key",
				color: "#8e8e93"
			}
		}, ...W.slice(5).map((e) => ({
			key: e.name,
			title: e.title,
			href: `#${e.name}`,
			app: e,
			badge: +(e.name === "devices")
		}))]
	}
];
function tt() {
	let t = e(8), [n, i] = r("#agent-devices"), a;
	t[0] === n ? a = t[1] : (a = $.map((e) => ({
		...e,
		selected: e.href === n,
		items: e.items.map((e) => ({
			...e,
			selected: e.href === n
		}))
	})), t[0] = n, t[1] = a);
	let o = a, s;
	t[2] === n ? s = t[3] : (s = $.flatMap(rt).find((e) => e.href === n)?.title ?? $.find((e) => e.href === n)?.title, t[2] = n, t[3] = s);
	let c = s, l;
	t[4] === Symbol.for("react.memo_cache_sentinel") ? (l = /* @__PURE__ */ L("div", {
		className: "p-4",
		children: /* @__PURE__ */ L(xe, {
			min: "10rem",
			children: W.slice(0, 4).map(nt)
		})
	}), t[4] = l) : l = t[4];
	let u;
	return t[5] !== o || t[6] !== c ? (u = /* @__PURE__ */ L(ze, {
		sections: o,
		title: c,
		onGo: i,
		children: l
	}), t[5] = o, t[6] = c, t[7] = u) : u = t[7], u;
}
function nt(e) {
	return /* @__PURE__ */ L(x, { className: "h-28" }, e.name);
}
function rt(e) {
	return e.items;
}
function it() {
	let t = e(55), n;
	t[0] === Symbol.for("react.memo_cache_sentinel") ? (n = /* @__PURE__ */ L("h1", {
		className: "text-2xl font-semibold tracking-tight",
		children: "@one/ui"
	}), t[0] = n) : n = t[0];
	let r;
	t[1] === Symbol.for("react.memo_cache_sentinel") ? (r = /* @__PURE__ */ R(Q, { children: [
		/* @__PURE__ */ L(b, { children: "Default" }),
		/* @__PURE__ */ L(b, {
			variant: "secondary",
			children: "Secondary"
		}),
		/* @__PURE__ */ L(b, {
			variant: "outline",
			children: "Outline"
		}),
		/* @__PURE__ */ L(b, {
			variant: "ghost",
			children: "Ghost"
		}),
		/* @__PURE__ */ L(b, {
			variant: "destructive",
			children: "Destructive"
		}),
		/* @__PURE__ */ L(b, {
			variant: "link",
			children: "Link"
		})
	] }), t[1] = r) : r = t[1];
	let i, a;
	t[2] === Symbol.for("react.memo_cache_sentinel") ? (i = /* @__PURE__ */ R(b, {
		size: "sm",
		variant: "outline",
		children: [/* @__PURE__ */ L(Ye, {}), " New Item"]
	}), a = /* @__PURE__ */ L(b, {
		size: "lg",
		children: "Large"
	}), t[2] = i, t[3] = a) : (i = t[2], a = t[3]);
	let C;
	t[4] === Symbol.for("react.memo_cache_sentinel") ? (C = /* @__PURE__ */ R(Z, {
		title: "Button: one default per view, then secondary, outline, ghost; destructive only for what can't be undone",
		children: [r, /* @__PURE__ */ R(Q, { children: [
			i,
			a,
			/* @__PURE__ */ L(b, {
				size: "icon",
				variant: "ghost",
				"aria-label": "More",
				children: /* @__PURE__ */ L(qe, { weight: "bold" })
			}),
			/* @__PURE__ */ L(b, {
				disabled: !0,
				children: "Disabled"
			})
		] })]
	}), t[4] = C) : C = t[4];
	let Se;
	t[5] === Symbol.for("react.memo_cache_sentinel") ? (Se = /* @__PURE__ */ L(Z, {
		title: "Badge",
		children: /* @__PURE__ */ R(Q, { children: [
			/* @__PURE__ */ L(y, { children: "Default" }),
			/* @__PURE__ */ L(y, {
				variant: "secondary",
				children: "Secondary"
			}),
			/* @__PURE__ */ L(y, {
				variant: "outline",
				children: "Outline"
			}),
			/* @__PURE__ */ L(y, {
				variant: "destructive",
				className: "rounded-full",
				children: "3"
			})
		] })
	}), t[5] = Se) : Se = t[5];
	let w;
	t[6] === Symbol.for("react.memo_cache_sentinel") ? (w = /* @__PURE__ */ L(Z, {
		title: "Input",
		children: /* @__PURE__ */ R("div", {
			className: "relative max-w-sm",
			children: [/* @__PURE__ */ L(Je, { className: "pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" }), /* @__PURE__ */ L(Ce, {
				placeholder: "Search",
				className: "pl-8"
			})]
		})
	}), t[6] = w) : w = t[6];
	let T;
	t[7] === Symbol.for("react.memo_cache_sentinel") ? (T = /* @__PURE__ */ L(Ae, {
		className: "w-48",
		"aria-label": "Device",
		children: /* @__PURE__ */ L(je, { placeholder: "Device" })
	}), t[7] = T) : T = t[7];
	let E;
	t[8] === Symbol.for("react.memo_cache_sentinel") ? (E = /* @__PURE__ */ L(Z, {
		title: "Select: a choice among a few things",
		children: /* @__PURE__ */ R(Oe, {
			defaultValue: "mac",
			children: [T, /* @__PURE__ */ R(ke, { children: [
				/* @__PURE__ */ L(A, {
					value: "mac",
					children: "Andre's Mac"
				}),
				/* @__PURE__ */ L(A, {
					value: "linux",
					children: "linux"
				}),
				/* @__PURE__ */ L(A, {
					value: "phone",
					children: "iPhone"
				})
			] })]
		})
	}), t[8] = E) : E = t[8];
	let D;
	t[9] === Symbol.for("react.memo_cache_sentinel") ? (D = /* @__PURE__ */ L(Z, {
		title: "Tabs",
		children: /* @__PURE__ */ R(Ue, {
			defaultValue: "you",
			className: "max-w-md",
			children: [
				/* @__PURE__ */ R(Ge, { children: [
					/* @__PURE__ */ L(Ke, {
						value: "you",
						children: "You"
					}),
					/* @__PURE__ */ L(Ke, {
						value: "team",
						children: "Team"
					}),
					/* @__PURE__ */ L(Ke, {
						value: "everyone",
						children: "Everyone"
					})
				] }),
				/* @__PURE__ */ L(We, {
					value: "you",
					className: "text-sm text-muted-foreground",
					children: "Yours alone."
				}),
				/* @__PURE__ */ L(We, {
					value: "team",
					className: "text-sm text-muted-foreground",
					children: "Your team's."
				}),
				/* @__PURE__ */ L(We, {
					value: "everyone",
					className: "text-sm text-muted-foreground",
					children: "Everyone's."
				})
			]
		})
	}), t[9] = D) : D = t[9];
	let O;
	t[10] === Symbol.for("react.memo_cache_sentinel") ? (O = /* @__PURE__ */ L(Z, {
		title: "Switch",
		children: /* @__PURE__ */ R(Q, { children: [
			/* @__PURE__ */ L(j, {
				defaultChecked: !0,
				"aria-label": "On"
			}),
			/* @__PURE__ */ L(j, { "aria-label": "Off" }),
			/* @__PURE__ */ L(j, {
				disabled: !0,
				"aria-label": "Disabled"
			})
		] })
	}), t[10] = O) : O = t[10];
	let k, De;
	t[11] === Symbol.for("react.memo_cache_sentinel") ? (k = /* @__PURE__ */ R(ne, { children: [/* @__PURE__ */ L(re, { children: "Backups" }), /* @__PURE__ */ L(ee, { children: "Encrypted copies of everything." })] }), De = /* @__PURE__ */ L(S, {
		className: "text-sm",
		children: "Last backed up 5 minutes ago."
	}), t[11] = k, t[12] = De) : (k = t[11], De = t[12]);
	let ze;
	t[13] === Symbol.for("react.memo_cache_sentinel") ? (ze = /* @__PURE__ */ L(Z, {
		title: "Card",
		children: /* @__PURE__ */ R(x, {
			className: "max-w-sm",
			children: [
				k,
				De,
				/* @__PURE__ */ L(te, { children: /* @__PURE__ */ L(b, {
					variant: "outline",
					size: "sm",
					children: "Back Up Now"
				}) })
			]
		})
	}), t[13] = ze) : ze = t[13];
	let He;
	t[14] === Symbol.for("react.memo_cache_sentinel") ? (He = /* @__PURE__ */ L(ye, {
		asChild: !0,
		children: /* @__PURE__ */ L(b, {
			variant: "outline",
			children: "Menu"
		})
	}), t[14] = He) : He = t[14];
	let M;
	t[15] === Symbol.for("react.memo_cache_sentinel") ? (M = /* @__PURE__ */ R(me, { children: [He, /* @__PURE__ */ R(he, { children: [
		/* @__PURE__ */ L(_e, { children: "Agent Devices" }),
		/* @__PURE__ */ L(ve, {}),
		/* @__PURE__ */ L(ge, { children: "Open" }),
		/* @__PURE__ */ L(ge, { children: "Rename" }),
		/* @__PURE__ */ L(ge, {
			variant: "destructive",
			children: "Delete"
		})
	] })] }), t[15] = M) : M = t[15];
	let N;
	t[16] === Symbol.for("react.memo_cache_sentinel") ? (N = /* @__PURE__ */ L(fe, {
		asChild: !0,
		children: /* @__PURE__ */ L(b, {
			variant: "outline",
			children: "Dialog"
		})
	}), t[16] = N) : N = t[16];
	let P, F;
	t[17] === Symbol.for("react.memo_cache_sentinel") ? (P = /* @__PURE__ */ R(ue, { children: [/* @__PURE__ */ L(de, { children: "Rename" }), /* @__PURE__ */ L(ce, { children: "What it's called on every device." })] }), F = /* @__PURE__ */ L(Ce, { defaultValue: "Anthropic" }), t[17] = P, t[18] = F) : (P = t[17], F = t[18]);
	let I;
	t[19] === Symbol.for("react.memo_cache_sentinel") ? (I = /* @__PURE__ */ R(oe, { children: [N, /* @__PURE__ */ R(se, { children: [
		P,
		F,
		/* @__PURE__ */ L(le, { children: /* @__PURE__ */ L(b, { children: "Save" }) })
	] })] }), t[19] = I) : I = t[19];
	let z;
	t[20] === Symbol.for("react.memo_cache_sentinel") ? (z = /* @__PURE__ */ L(m, {
		asChild: !0,
		children: /* @__PURE__ */ L(b, {
			variant: "outline",
			children: "Alert dialog"
		})
	}), t[20] = z) : z = t[20];
	let B;
	t[21] === Symbol.for("react.memo_cache_sentinel") ? (B = /* @__PURE__ */ R(f, { children: [/* @__PURE__ */ L(p, { children: "Delete Anthropic?" }), /* @__PURE__ */ L(u, { children: "Apps using it stop working until you add another." })] }), t[21] = B) : B = t[21];
	let V;
	t[22] === Symbol.for("react.memo_cache_sentinel") ? (V = /* @__PURE__ */ R(o, { children: [z, /* @__PURE__ */ R(l, { children: [B, /* @__PURE__ */ R(d, { children: [/* @__PURE__ */ L(c, { children: "Cancel" }), /* @__PURE__ */ L(s, {
		className: "bg-destructive text-white hover:bg-destructive/90",
		children: "Delete"
	})] })] })] }), t[22] = V) : V = t[22];
	let H;
	t[23] === Symbol.for("react.memo_cache_sentinel") ? (H = /* @__PURE__ */ L(Re, {
		asChild: !0,
		children: /* @__PURE__ */ L(b, {
			variant: "outline",
			children: "Sheet"
		})
	}), t[23] = H) : H = t[23];
	let U;
	t[24] === Symbol.for("react.memo_cache_sentinel") ? (U = /* @__PURE__ */ L(Z, {
		title: "Dropdown menu, dialog, alert dialog, sheet",
		children: /* @__PURE__ */ R(Q, { children: [
			M,
			I,
			V,
			/* @__PURE__ */ R(Ne, { children: [H, /* @__PURE__ */ L(Pe, { children: /* @__PURE__ */ R(Ie, { children: [/* @__PURE__ */ L(Le, { children: "Anthropic" }), /* @__PURE__ */ L(Fe, { children: "Key · Used by AI and Models" })] }) })] })
		] })
	}), t[24] = U) : U = t[24];
	let G;
	t[25] === Symbol.for("react.memo_cache_sentinel") ? (G = /* @__PURE__ */ L(Z, {
		title: "Separator and skeleton",
		children: /* @__PURE__ */ R("div", {
			className: "flex max-w-sm flex-col gap-3",
			children: [
				/* @__PURE__ */ L(Be, { className: "h-4 w-3/4" }),
				/* @__PURE__ */ L(Be, { className: "h-4 w-1/2" }),
				/* @__PURE__ */ L(Me, {}),
				/* @__PURE__ */ L(Be, { className: "aspect-[16/10] w-full rounded-xl" })
			]
		})
	}), t[25] = G) : G = t[25];
	let K;
	t[26] === Symbol.for("react.memo_cache_sentinel") ? (K = /* @__PURE__ */ L(g, {
		size: "sm",
		children: /* @__PURE__ */ L(_, { children: "AL" })
	}), t[26] = K) : K = t[26];
	let q;
	t[27] === Symbol.for("react.memo_cache_sentinel") ? (q = /* @__PURE__ */ R(g, { children: [/* @__PURE__ */ L(v, {
		src: "/icons/missing.png",
		alt: ""
	}), /* @__PURE__ */ L(_, { children: "AL" })] }), t[27] = q) : q = t[27];
	let J;
	t[28] === Symbol.for("react.memo_cache_sentinel") ? (J = /* @__PURE__ */ L(Z, {
		title: "Avatar: a circle with a picture, or the initials of a name, grey or in a colour that stays the same for an id",
		children: /* @__PURE__ */ R(Q, { children: [
			K,
			q,
			/* @__PURE__ */ L(g, {
				size: "lg",
				children: /* @__PURE__ */ L(_, { children: "AL" })
			}),
			/* @__PURE__ */ L(g, {
				name: "Ann Lee",
				size: 20
			}),
			/* @__PURE__ */ L(g, {
				name: "Ann Lee",
				size: 32
			}),
			/* @__PURE__ */ L(g, {
				name: "Bo Chen",
				id: "u1",
				size: 36
			}),
			/* @__PURE__ */ L(g, {
				name: "Cy Dunn",
				id: "u2",
				size: 36
			}),
			/* @__PURE__ */ L(g, {
				name: "Di Eve",
				id: "u3",
				size: 36
			}),
			/* @__PURE__ */ L(g, {
				name: "Ed Fox",
				id: "u4",
				size: 36
			})
		] })
	}), t[28] = J) : J = t[28];
	let Y, X, $, nt;
	t[29] === Symbol.for("react.memo_cache_sentinel") ? (Y = /* @__PURE__ */ L(Ee, { value: 40 }), X = /* @__PURE__ */ L(Ee, {
		value: 40,
		meter: !0
	}), $ = /* @__PURE__ */ L(Ee, {
		value: 75,
		meter: !0
	}), nt = /* @__PURE__ */ L(Ee, {
		value: 92,
		meter: !0
	}), t[29] = Y, t[30] = X, t[31] = $, t[32] = nt) : (Y = t[29], X = t[30], $ = t[31], nt = t[32]);
	let rt;
	t[33] === Symbol.for("react.memo_cache_sentinel") ? (rt = /* @__PURE__ */ L(Ve, {
		values: [
			10,
			14,
			12,
			30,
			44,
			38,
			52,
			47,
			60,
			41,
			35,
			50
		],
		className: "text-foreground"
	}), t[33] = rt) : rt = t[33];
	let it;
	t[34] === Symbol.for("react.memo_cache_sentinel") ? (it = /* @__PURE__ */ L(Ve, {
		values: [
			5,
			8,
			6,
			20,
			12,
			9
		],
		className: "text-muted-foreground"
	}), t[34] = it) : it = t[34];
	let mt;
	t[35] === Symbol.for("react.memo_cache_sentinel") ? (mt = /* @__PURE__ */ L(Z, {
		title: "Progress: a bar filled to a percent; meter, for what's used, turns amber from 70 and red from 85. Sparkline: a small line of how something changed",
		children: /* @__PURE__ */ R("div", {
			className: "grid max-w-md gap-3",
			children: [
				Y,
				X,
				$,
				nt,
				rt,
				it,
				/* @__PURE__ */ L(Ve, { values: [] })
			]
		})
	}), t[35] = mt) : mt = t[35];
	let ht;
	t[36] === Symbol.for("react.memo_cache_sentinel") ? (ht = /* @__PURE__ */ L(Te, {
		title: "Couldn't reach your devices.",
		text: "Check your connection.",
		action: {
			label: "Try Again",
			run: pt
		}
	}), t[36] = ht) : ht = t[36];
	let gt;
	t[37] === Symbol.for("react.memo_cache_sentinel") ? (gt = /* @__PURE__ */ R(Z, {
		title: "Problem: on the thing it's about, with the action that fixes it",
		children: [ht, /* @__PURE__ */ L(Te, {
			warn: !0,
			title: "None of your devices runs Agent Devices.",
			text: "It needs a Mac, or a Linux device with Podman.",
			action: {
				label: "Open App Store",
				run: ft
			}
		})]
	}), t[37] = gt) : gt = t[37];
	let _t;
	t[38] === Symbol.for("react.memo_cache_sentinel") ? (_t = /* @__PURE__ */ L(Z, {
		title: "Empty: one grey line",
		children: /* @__PURE__ */ L(x, { children: /* @__PURE__ */ L(be, { children: "None yet. Agents make one when they need a browser." }) })
	}), t[38] = _t) : _t = t[38];
	let vt;
	t[39] === Symbol.for("react.memo_cache_sentinel") ? (vt = W.map(dt), t[39] = vt) : vt = t[39];
	let yt;
	t[40] === Symbol.for("react.memo_cache_sentinel") ? (yt = /* @__PURE__ */ R(Q, { children: [vt, /* @__PURE__ */ L(h, { app: {
		name: "x",
		title: "Xylophone"
	} })] }), t[40] = yt) : yt = t[40];
	let bt;
	t[41] === Symbol.for("react.memo_cache_sentinel") ? (bt = /* @__PURE__ */ R(Z, {
		title: "App icons, in each app's colour, and icons by name",
		children: [yt, /* @__PURE__ */ L(Q, { children: [
			"magnifying-glass",
			"gear",
			"warning-circle-fill",
			"check-circle-fill",
			"browsers"
		].map(ut) })]
	}), t[41] = bt) : bt = t[41];
	let xt;
	t[42] === Symbol.for("react.memo_cache_sentinel") ? (xt = /* @__PURE__ */ L(Z, {
		title: "Grid: fills the room, at most 4 across here",
		children: /* @__PURE__ */ L(xe, {
			min: "12rem",
			max: 4,
			children: W.slice(0, 6).map(lt)
		})
	}), t[42] = xt) : xt = t[42];
	let St;
	t[43] === Symbol.for("react.memo_cache_sentinel") ? (St = /* @__PURE__ */ L(Z, {
		title: "List: rows that stop at a readable width",
		children: /* @__PURE__ */ L(we, { children: W.slice(0, 4).map(ct) })
	}), t[43] = St) : St = t[43];
	let Ct;
	t[44] === Symbol.for("react.memo_cache_sentinel") ? (Ct = /* @__PURE__ */ L(Z, {
		title: "ListRow's wrap: a long line runs on instead of being cut; useDialog opens a dialog afresh each time",
		children: /* @__PURE__ */ L($e, {})
	}), t[44] = Ct) : Ct = t[44];
	let wt;
	t[45] === Symbol.for("react.memo_cache_sentinel") ? (wt = /* @__PURE__ */ L(Z, {
		title: "Accounts: AccountList (rows with Sign Out), LoadingButton, Select in a row, ListNote, Hint, Loading, ListSkeleton, LoadFailure; useAccounts keeps them fresh from an app's accounts and sign_out calls",
		children: /* @__PURE__ */ L(Qe, {})
	}), t[45] = wt) : wt = t[45];
	let Tt;
	t[46] === Symbol.for("react.memo_cache_sentinel") ? (Tt = /* @__PURE__ */ L(Z, {
		title: "Split: what a list opens shows beside it where there's room, else in a sheet",
		children: /* @__PURE__ */ L(et, {})
	}), t[46] = Tt) : Tt = t[46];
	let Et;
	t[47] === Symbol.for("react.memo_cache_sentinel") ? (Et = /* @__PURE__ */ R("div", {
		className: "grid grid-cols-1 items-start gap-4 lg:grid-cols-[minmax(0,1fr)_22rem]",
		children: [/* @__PURE__ */ L(ie, {
			text: Xe,
			path: "recall.ts"
		}), /* @__PURE__ */ L(ie, {
			text: Xe,
			path: "recall.ts"
		})]
	}), t[47] = Et) : Et = t[47];
	let Dt;
	t[48] === Symbol.for("react.memo_cache_sentinel") ? (Dt = /* @__PURE__ */ L(x, {
		className: "overflow-hidden py-0",
		children: /* @__PURE__ */ L(pe, {
			hunks: Ze,
			path: "recall.ts"
		})
	}), t[48] = Dt) : Dt = t[48];
	let Ot;
	t[49] === Symbol.for("react.memo_cache_sentinel") ? (Ot = /* @__PURE__ */ R("div", {
		className: "grid grid-cols-1 items-start gap-4 lg:grid-cols-[minmax(0,1fr)_22rem]",
		children: [Dt, /* @__PURE__ */ L(x, {
			className: "overflow-hidden py-0",
			children: /* @__PURE__ */ L(pe, {
				hunks: Ze,
				path: "recall.ts"
			})
		})]
	}), t[49] = Ot) : Ot = t[49];
	let kt;
	t[50] === Symbol.for("react.memo_cache_sentinel") ? (kt = /* @__PURE__ */ R(Z, {
		title: "CodeView: a file, read only, with Find, Wrap and Copy; DiffView: its changes, side by side where there's room. Each sizes itself by its own width",
		children: [
			Et,
			Ot,
			/* @__PURE__ */ R(Q, { children: [/* @__PURE__ */ L(ae, { text: "npm run build" }), /* @__PURE__ */ L(ae, {
				text: "9fceb02",
				label: ""
			})] })
		]
	}), t[50] = kt) : kt = t[50];
	let At;
	t[51] === Symbol.for("react.memo_cache_sentinel") ? (At = { gridTemplateColumns: "minmax(0, 1fr) 22rem" }, t[51] = At) : At = t[51];
	let jt;
	t[52] === Symbol.for("react.memo_cache_sentinel") ? (jt = /* @__PURE__ */ L("div", {
		className: "h-[30rem] overflow-hidden rounded-xl border",
		children: /* @__PURE__ */ L(tt, {})
	}), t[52] = jt) : jt = t[52];
	let Mt;
	t[53] === Symbol.for("react.memo_cache_sentinel") ? (Mt = /* @__PURE__ */ L(Z, {
		title: "Shell: the window every client shows screens in; where its own room is narrow, the sidebar is a sheet behind the bar's button",
		children: /* @__PURE__ */ R("div", {
			className: "grid gap-4",
			style: At,
			children: [jt, /* @__PURE__ */ L("div", {
				className: "h-[30rem] overflow-hidden rounded-xl border",
				children: /* @__PURE__ */ L(tt, {})
			})]
		})
	}), t[53] = Mt) : Mt = t[53];
	let Nt;
	return t[54] === Symbol.for("react.memo_cache_sentinel") ? (Nt = /* @__PURE__ */ R("main", {
		className: "mx-auto flex flex-col gap-10",
		children: [
			n,
			C,
			Se,
			w,
			E,
			D,
			O,
			ze,
			U,
			G,
			J,
			mt,
			gt,
			_t,
			bt,
			xt,
			St,
			Ct,
			wt,
			Tt,
			kt,
			Mt,
			/* @__PURE__ */ L(Z, {
				title: "What the window shows over screens when they ask: a menu, an alert, a screen in a sheet",
				children: /* @__PURE__ */ R(Q, { children: [
					/* @__PURE__ */ L(b, {
						variant: "outline",
						onClick: st,
						children: "Menu"
					}),
					/* @__PURE__ */ L(b, {
						variant: "outline",
						onClick: ot,
						children: "Alert"
					}),
					/* @__PURE__ */ L(b, {
						variant: "outline",
						onClick: at,
						children: "Sheet"
					})
				] })
			})
		]
	}), t[54] = Nt) : Nt = t[54], Nt;
}
function at() {
	let e = P({ title: "Vault" });
	e.body.innerHTML = "<p style=\"padding:1rem\">A screen goes here, in its frame.</p>";
}
function ot() {
	return M({
		title: "Remove Repos?",
		text: "Its data stays until you erase it.",
		action: "Remove",
		danger: !0
	});
}
function st(e) {
	let t = e.currentTarget.getBoundingClientRect();
	N([
		{
			title: "Small",
			icon: "square",
			row: !0,
			checked: !0
		},
		{
			title: "Wide",
			icon: "rectangle",
			row: !0
		},
		{ title: "Rename…" },
		{ title: "Move to Everyone" },
		{
			title: "Remove",
			danger: !0,
			section: !0
		}
	], [
		t.left,
		t.top,
		t.width,
		t.height
	]);
}
function ct(e) {
	return /* @__PURE__ */ L(T, {
		leading: /* @__PURE__ */ L(h, {
			app: e,
			size: 28
		}),
		title: e.title,
		sub: "On every device",
		trailing: e.name === "devices" ? /* @__PURE__ */ L(y, {
			variant: "destructive",
			className: "rounded-full",
			children: "1"
		}) : void 0
	}, e.name);
}
function lt(e) {
	return /* @__PURE__ */ R("div", {
		className: "min-w-0",
		children: [
			/* @__PURE__ */ L("div", {
				className: "grid aspect-[16/10] place-items-center rounded-xl bg-muted",
				children: /* @__PURE__ */ L(h, {
					app: e,
					size: 48
				})
			}),
			/* @__PURE__ */ L("div", {
				className: "mt-2 truncate text-sm font-medium",
				children: e.title
			}),
			/* @__PURE__ */ L("div", {
				className: "truncate text-sm text-muted-foreground",
				children: "personal"
			})
		]
	}, e.name);
}
function ut(e) {
	return /* @__PURE__ */ L(Se, {
		name: e,
		className: "size-5"
	}, e);
}
function dt(e) {
	return /* @__PURE__ */ L(h, { app: e }, e.name);
}
function ft() {}
function pt() {}
i(document.getElementById("root")).render(/* @__PURE__ */ L(it, {}));
