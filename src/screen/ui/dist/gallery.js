import { c as e } from "react/compiler-runtime";
import * as t from "react";
import { createContext as n, useState as r } from "react";
import { createRoot as i } from "react-dom/client";
import { AlertDialog as a, AlertDialogAction as o, AlertDialogCancel as s, AlertDialogContent as c, AlertDialogDescription as l, AlertDialogFooter as u, AlertDialogHeader as d, AlertDialogTitle as f, AlertDialogTrigger as ee, AppIcon as p, Badge as m, Button as h, Card as g, CardContent as te, CardDescription as ne, CardFooter as re, CardHeader as ie, CardTitle as ae, Dialog as oe, DialogContent as se, DialogDescription as ce, DialogFooter as le, DialogHeader as ue, DialogTitle as de, DialogTrigger as fe, DropdownMenu as pe, DropdownMenuContent as me, DropdownMenuItem as he, DropdownMenuLabel as ge, DropdownMenuSeparator as _e, DropdownMenuTrigger as ve, Empty as ye, Grid as be, Icon as _, Input as xe, List as Se, ListRow as v, Problem as Ce, Separator as we, Sheet as Te, SheetContent as Ee, SheetDescription as De, SheetHeader as Oe, SheetTitle as ke, SheetTrigger as Ae, Shell as y, Skeleton as je, Split as b, Switch as x, Tabs as Me, TabsContent as Ne, TabsList as Pe, TabsTrigger as Fe, showAlert as S, showMenu as C, showSheet as w } from "@one/ui";
import { jsx as T, jsxs as E } from "react/jsx-runtime";
// node_modules/@phosphor-icons/react/dist/defs/DotsThree.es.js
var D = /* @__PURE__ */ new Map([
	["bold", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M144,128a16,16,0,1,1-16-16A16,16,0,0,1,144,128ZM60,112a16,16,0,1,0,16,16A16,16,0,0,0,60,112Zm136,0a16,16,0,1,0,16,16A16,16,0,0,0,196,112Z" }))],
	["duotone", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", {
		d: "M240,96v64a16,16,0,0,1-16,16H32a16,16,0,0,1-16-16V96A16,16,0,0,1,32,80H224A16,16,0,0,1,240,96Z",
		opacity: "0.2"
	}), /* @__PURE__ */ t.createElement("path", { d: "M140,128a12,12,0,1,1-12-12A12,12,0,0,1,140,128Zm56-12a12,12,0,1,0,12,12A12,12,0,0,0,196,116ZM60,116a12,12,0,1,0,12,12A12,12,0,0,0,60,116Z" }))],
	["fill", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M224,80H32A16,16,0,0,0,16,96v64a16,16,0,0,0,16,16H224a16,16,0,0,0,16-16V96A16,16,0,0,0,224,80ZM60,140a12,12,0,1,1,12-12A12,12,0,0,1,60,140Zm68,0a12,12,0,1,1,12-12A12,12,0,0,1,128,140Zm68,0a12,12,0,1,1,12-12A12,12,0,0,1,196,140Z" }))],
	["light", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M138,128a10,10,0,1,1-10-10A10,10,0,0,1,138,128ZM60,118a10,10,0,1,0,10,10A10,10,0,0,0,60,118Zm136,0a10,10,0,1,0,10,10A10,10,0,0,0,196,118Z" }))],
	["regular", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M140,128a12,12,0,1,1-12-12A12,12,0,0,1,140,128Zm56-12a12,12,0,1,0,12,12A12,12,0,0,0,196,116ZM60,116a12,12,0,1,0,12,12A12,12,0,0,0,60,116Z" }))],
	["thin", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M136,128a8,8,0,1,1-8-8A8,8,0,0,1,136,128Zm-76-8a8,8,0,1,0,8,8A8,8,0,0,0,60,120Zm136,0a8,8,0,1,0,8,8A8,8,0,0,0,196,120Z" }))]
]), O = /* @__PURE__ */ new Map([
	["bold", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M232.49,215.51,185,168a92.12,92.12,0,1,0-17,17l47.53,47.54a12,12,0,0,0,17-17ZM44,112a68,68,0,1,1,68,68A68.07,68.07,0,0,1,44,112Z" }))],
	["duotone", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", {
		d: "M192,112a80,80,0,1,1-80-80A80,80,0,0,1,192,112Z",
		opacity: "0.2"
	}), /* @__PURE__ */ t.createElement("path", { d: "M229.66,218.34,179.6,168.28a88.21,88.21,0,1,0-11.32,11.31l50.06,50.07a8,8,0,0,0,11.32-11.32ZM40,112a72,72,0,1,1,72,72A72.08,72.08,0,0,1,40,112Z" }))],
	["fill", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M168,112a56,56,0,1,1-56-56A56,56,0,0,1,168,112Zm61.66,117.66a8,8,0,0,1-11.32,0l-50.06-50.07a88,88,0,1,1,11.32-11.31l50.06,50.06A8,8,0,0,1,229.66,229.66ZM112,184a72,72,0,1,0-72-72A72.08,72.08,0,0,0,112,184Z" }))],
	["light", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M228.24,219.76l-51.38-51.38a86.15,86.15,0,1,0-8.48,8.48l51.38,51.38a6,6,0,0,0,8.48-8.48ZM38,112a74,74,0,1,1,74,74A74.09,74.09,0,0,1,38,112Z" }))],
	["regular", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M229.66,218.34l-50.07-50.06a88.11,88.11,0,1,0-11.31,11.31l50.06,50.07a8,8,0,0,0,11.32-11.32ZM40,112a72,72,0,1,1,72,72A72.08,72.08,0,0,1,40,112Z" }))],
	["thin", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M226.83,221.17l-52.7-52.7a84.1,84.1,0,1,0-5.66,5.66l52.7,52.7a4,4,0,0,0,5.66-5.66ZM36,112a76,76,0,1,1,76,76A76.08,76.08,0,0,1,36,112Z" }))]
]), k = /* @__PURE__ */ new Map([
	["bold", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M228,128a12,12,0,0,1-12,12H140v76a12,12,0,0,1-24,0V140H40a12,12,0,0,1,0-24h76V40a12,12,0,0,1,24,0v76h76A12,12,0,0,1,228,128Z" }))],
	["duotone", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", {
		d: "M216,56V200a16,16,0,0,1-16,16H56a16,16,0,0,1-16-16V56A16,16,0,0,1,56,40H200A16,16,0,0,1,216,56Z",
		opacity: "0.2"
	}), /* @__PURE__ */ t.createElement("path", { d: "M224,128a8,8,0,0,1-8,8H136v80a8,8,0,0,1-16,0V136H40a8,8,0,0,1,0-16h80V40a8,8,0,0,1,16,0v80h80A8,8,0,0,1,224,128Z" }))],
	["fill", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M208,32H48A16,16,0,0,0,32,48V208a16,16,0,0,0,16,16H208a16,16,0,0,0,16-16V48A16,16,0,0,0,208,32ZM184,136H136v48a8,8,0,0,1-16,0V136H72a8,8,0,0,1,0-16h48V72a8,8,0,0,1,16,0v48h48a8,8,0,0,1,0,16Z" }))],
	["light", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M222,128a6,6,0,0,1-6,6H134v82a6,6,0,0,1-12,0V134H40a6,6,0,0,1,0-12h82V40a6,6,0,0,1,12,0v82h82A6,6,0,0,1,222,128Z" }))],
	["regular", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M224,128a8,8,0,0,1-8,8H136v80a8,8,0,0,1-16,0V136H40a8,8,0,0,1,0-16h80V40a8,8,0,0,1,16,0v80h80A8,8,0,0,1,224,128Z" }))],
	["thin", /* @__PURE__ */ t.createElement(t.Fragment, null, /* @__PURE__ */ t.createElement("path", { d: "M220,128a4,4,0,0,1-4,4H132v84a4,4,0,0,1-8,0V132H40a4,4,0,0,1,0-8h84V40a4,4,0,0,1,8,0v84h84A4,4,0,0,1,220,128Z" }))]
]), A = n({
	color: "currentColor",
	size: "1em",
	weight: "regular",
	mirrored: !1
}), j = t.forwardRef((e, n) => {
	let { alt: r, color: i, size: a, weight: o, mirrored: s, children: c, weights: l, ...u } = e, { color: d = "currentColor", size: f, weight: ee = "regular", mirrored: p = !1, ...m } = t.useContext(A);
	return /* @__PURE__ */ t.createElement("svg", {
		ref: n,
		xmlns: "http://www.w3.org/2000/svg",
		width: a ?? f,
		height: a ?? f,
		fill: i ?? d,
		viewBox: "0 0 256 256",
		transform: s || p ? "scale(-1, 1)" : void 0,
		...m,
		...u
	}, !!r && /* @__PURE__ */ t.createElement("title", null, r), c, l.get(o ?? ee));
});
j.displayName = "IconBase";
// node_modules/@phosphor-icons/react/dist/csr/DotsThree.es.js
var Ie = t.forwardRef((e, n) => /* @__PURE__ */ t.createElement(j, {
	ref: n,
	...e,
	weights: D
}));
Ie.displayName = "DotsThreeIcon";
// node_modules/@phosphor-icons/react/dist/csr/MagnifyingGlass.es.js
var Le = t.forwardRef((e, n) => /* @__PURE__ */ t.createElement(j, {
	ref: n,
	...e,
	weights: O
}));
Le.displayName = "MagnifyingGlassIcon";
// node_modules/@phosphor-icons/react/dist/csr/Plus.es.js
var Re = t.forwardRef((e, n) => /* @__PURE__ */ t.createElement(j, {
	ref: n,
	...e,
	weights: k
}));
Re.displayName = "PlusIcon";
// gallery/gallery.tsx
var M = [
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
], N = [
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
];
function P(t) {
	let n = e(5), { title: r, children: i } = t, a;
	n[0] === r ? a = n[1] : (a = /* @__PURE__ */ T("h2", {
		className: "text-sm font-medium text-muted-foreground",
		children: r
	}), n[0] = r, n[1] = a);
	let o;
	return n[2] !== i || n[3] !== a ? (o = /* @__PURE__ */ E("section", {
		className: "flex flex-col gap-3",
		children: [a, i]
	}), n[2] = i, n[3] = a, n[4] = o) : o = n[4], o;
}
function F(t) {
	let n = e(2), { children: r } = t, i;
	return n[0] === r ? i = n[1] : (i = /* @__PURE__ */ T("div", {
		className: "flex flex-wrap items-center gap-2",
		children: r
	}), n[0] = r, n[1] = i), i;
}
function ze() {
	let t = e(13), [n, i] = r(null), a;
	t[0] === n ? a = t[1] : (a = N.find((e) => e.id === n), t[0] = n, t[1] = a);
	let o = a, s = o?.title, c;
	t[2] === Symbol.for("react.memo_cache_sentinel") ? (c = () => i(null), t[2] = c) : c = t[2];
	let l;
	t[3] === n ? l = t[4] : (l = N.map((e) => /* @__PURE__ */ T(v, {
		leading: /* @__PURE__ */ T(p, {
			app: e.app,
			size: 32
		}),
		title: e.title,
		sub: e.sub,
		selected: e.id === n,
		onClick: () => i(e.id)
	}, e.id)), t[3] = n, t[4] = l);
	let u;
	t[5] === l ? u = t[6] : (u = /* @__PURE__ */ T(Se, { children: l }), t[5] = l, t[6] = u);
	let d;
	t[7] === o ? d = t[8] : (d = o && /* @__PURE__ */ E(g, { children: [
		/* @__PURE__ */ E(ie, {
			className: "flex flex-row items-center gap-4",
			children: [
				/* @__PURE__ */ T(p, {
					app: o.app,
					size: 56
				}),
				/* @__PURE__ */ E("div", {
					className: "min-w-0 flex-1",
					children: [/* @__PURE__ */ T(ae, {
						className: "text-lg",
						children: o.title
					}), /* @__PURE__ */ T(ne, { children: "You" })]
				}),
				/* @__PURE__ */ T(h, {
					variant: "outline",
					size: "sm",
					children: "Rename"
				})
			]
		}),
		/* @__PURE__ */ E(te, {
			className: "flex flex-col gap-3",
			children: [/* @__PURE__ */ E("div", {
				className: "flex items-center justify-between gap-3",
				children: [/* @__PURE__ */ T("span", {
					className: "text-sm",
					children: "AI"
				}), /* @__PURE__ */ T(x, { defaultChecked: !0 })]
			}), /* @__PURE__ */ E("div", {
				className: "flex items-center justify-between gap-3",
				children: [/* @__PURE__ */ T("span", {
					className: "text-sm",
					children: "AGENTS.md"
				}), /* @__PURE__ */ T(x, {})]
			})]
		}),
		/* @__PURE__ */ T(re, { children: /* @__PURE__ */ T(h, {
			variant: "destructive",
			size: "sm",
			children: "Delete Item"
		}) })
	] }), t[7] = o, t[8] = d);
	let f;
	return t[9] !== s || t[10] !== u || t[11] !== d ? (f = /* @__PURE__ */ T(b, {
		title: s,
		onClose: c,
		list: u,
		detail: d
	}), t[9] = s, t[10] = u, t[11] = d, t[12] = f) : f = t[12], f;
}
var I = [
	{
		key: "user",
		title: "You",
		href: "#you",
		items: [...M.slice(0, 1).map((e) => ({
			key: e.name,
			title: e.title,
			href: `#${e.name}`,
			app: e,
			selected: !0
		})), {
			key: "services",
			title: "Services",
			href: "#services",
			folder: M.slice(4, 6)
		}]
	},
	{
		key: "global",
		title: "Everyone",
		href: "#everyone",
		items: M.slice(1, 4).map((e) => ({
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
		}, ...M.slice(5).map((e) => ({
			key: e.name,
			title: e.title,
			href: `#${e.name}`,
			app: e,
			badge: +(e.name === "devices")
		}))]
	}
];
function Be() {
	let t = e(8), [n, i] = r("#agent-devices"), a;
	t[0] === n ? a = t[1] : (a = I.map((e) => ({
		...e,
		selected: e.href === n,
		items: e.items.map((e) => ({
			...e,
			selected: e.href === n
		}))
	})), t[0] = n, t[1] = a);
	let o = a, s;
	t[2] === n ? s = t[3] : (s = I.flatMap(R).find((e) => e.href === n)?.title ?? I.find((e) => e.href === n)?.title, t[2] = n, t[3] = s);
	let c = s, l;
	t[4] === Symbol.for("react.memo_cache_sentinel") ? (l = /* @__PURE__ */ T("div", {
		className: "p-4",
		children: /* @__PURE__ */ T(be, {
			min: "10rem",
			children: M.slice(0, 4).map(L)
		})
	}), t[4] = l) : l = t[4];
	let u;
	return t[5] !== o || t[6] !== c ? (u = /* @__PURE__ */ T(y, {
		sections: o,
		title: c,
		onGo: i,
		children: l
	}), t[5] = o, t[6] = c, t[7] = u) : u = t[7], u;
}
function L(e) {
	return /* @__PURE__ */ T(g, { className: "h-28" }, e.name);
}
function R(e) {
	return e.items;
}
function z() {
	let t = e(37), n;
	t[0] === Symbol.for("react.memo_cache_sentinel") ? (n = /* @__PURE__ */ T("h1", {
		className: "text-2xl font-semibold tracking-tight",
		children: "@one/ui"
	}), t[0] = n) : n = t[0];
	let r;
	t[1] === Symbol.for("react.memo_cache_sentinel") ? (r = /* @__PURE__ */ E(F, { children: [
		/* @__PURE__ */ T(h, { children: "Default" }),
		/* @__PURE__ */ T(h, {
			variant: "secondary",
			children: "Secondary"
		}),
		/* @__PURE__ */ T(h, {
			variant: "outline",
			children: "Outline"
		}),
		/* @__PURE__ */ T(h, {
			variant: "ghost",
			children: "Ghost"
		}),
		/* @__PURE__ */ T(h, {
			variant: "destructive",
			children: "Destructive"
		}),
		/* @__PURE__ */ T(h, {
			variant: "link",
			children: "Link"
		})
	] }), t[1] = r) : r = t[1];
	let i, _;
	t[2] === Symbol.for("react.memo_cache_sentinel") ? (i = /* @__PURE__ */ E(h, {
		size: "sm",
		variant: "outline",
		children: [/* @__PURE__ */ T(Re, {}), " New Item"]
	}), _ = /* @__PURE__ */ T(h, {
		size: "lg",
		children: "Large"
	}), t[2] = i, t[3] = _) : (i = t[2], _ = t[3]);
	let v;
	t[4] === Symbol.for("react.memo_cache_sentinel") ? (v = /* @__PURE__ */ E(P, {
		title: "Button: one default per view, then secondary, outline, ghost; destructive only for what can't be undone",
		children: [r, /* @__PURE__ */ E(F, { children: [
			i,
			_,
			/* @__PURE__ */ T(h, {
				size: "icon",
				variant: "ghost",
				"aria-label": "More",
				children: /* @__PURE__ */ T(Ie, { weight: "bold" })
			}),
			/* @__PURE__ */ T(h, {
				disabled: !0,
				children: "Disabled"
			})
		] })]
	}), t[4] = v) : v = t[4];
	let y;
	t[5] === Symbol.for("react.memo_cache_sentinel") ? (y = /* @__PURE__ */ T(P, {
		title: "Badge",
		children: /* @__PURE__ */ E(F, { children: [
			/* @__PURE__ */ T(m, { children: "Default" }),
			/* @__PURE__ */ T(m, {
				variant: "secondary",
				children: "Secondary"
			}),
			/* @__PURE__ */ T(m, {
				variant: "outline",
				children: "Outline"
			}),
			/* @__PURE__ */ T(m, {
				variant: "destructive",
				className: "rounded-full",
				children: "3"
			})
		] })
	}), t[5] = y) : y = t[5];
	let b;
	t[6] === Symbol.for("react.memo_cache_sentinel") ? (b = /* @__PURE__ */ T(P, {
		title: "Input",
		children: /* @__PURE__ */ E("div", {
			className: "relative max-w-sm",
			children: [/* @__PURE__ */ T(Le, { className: "pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" }), /* @__PURE__ */ T(xe, {
				placeholder: "Search",
				className: "pl-8"
			})]
		})
	}), t[6] = b) : b = t[6];
	let S;
	t[7] === Symbol.for("react.memo_cache_sentinel") ? (S = /* @__PURE__ */ T(P, {
		title: "Tabs",
		children: /* @__PURE__ */ E(Me, {
			defaultValue: "you",
			className: "max-w-md",
			children: [
				/* @__PURE__ */ E(Pe, { children: [
					/* @__PURE__ */ T(Fe, {
						value: "you",
						children: "You"
					}),
					/* @__PURE__ */ T(Fe, {
						value: "team",
						children: "Team"
					}),
					/* @__PURE__ */ T(Fe, {
						value: "everyone",
						children: "Everyone"
					})
				] }),
				/* @__PURE__ */ T(Ne, {
					value: "you",
					className: "text-sm text-muted-foreground",
					children: "Yours alone."
				}),
				/* @__PURE__ */ T(Ne, {
					value: "team",
					className: "text-sm text-muted-foreground",
					children: "Your team's."
				}),
				/* @__PURE__ */ T(Ne, {
					value: "everyone",
					className: "text-sm text-muted-foreground",
					children: "Everyone's."
				})
			]
		})
	}), t[7] = S) : S = t[7];
	let C;
	t[8] === Symbol.for("react.memo_cache_sentinel") ? (C = /* @__PURE__ */ T(P, {
		title: "Switch",
		children: /* @__PURE__ */ E(F, { children: [
			/* @__PURE__ */ T(x, {
				defaultChecked: !0,
				"aria-label": "On"
			}),
			/* @__PURE__ */ T(x, { "aria-label": "Off" }),
			/* @__PURE__ */ T(x, {
				disabled: !0,
				"aria-label": "Disabled"
			})
		] })
	}), t[8] = C) : C = t[8];
	let w, D;
	t[9] === Symbol.for("react.memo_cache_sentinel") ? (D = /* @__PURE__ */ E(ie, { children: [/* @__PURE__ */ T(ae, { children: "Backups" }), /* @__PURE__ */ T(ne, { children: "Encrypted copies of everything." })] }), w = /* @__PURE__ */ T(te, {
		className: "text-sm",
		children: "Last backed up 5 minutes ago."
	}), t[9] = w, t[10] = D) : (w = t[9], D = t[10]);
	let O;
	t[11] === Symbol.for("react.memo_cache_sentinel") ? (O = /* @__PURE__ */ T(P, {
		title: "Card",
		children: /* @__PURE__ */ E(g, {
			className: "max-w-sm",
			children: [
				D,
				w,
				/* @__PURE__ */ T(re, { children: /* @__PURE__ */ T(h, {
					variant: "outline",
					size: "sm",
					children: "Back Up Now"
				}) })
			]
		})
	}), t[11] = O) : O = t[11];
	let k;
	t[12] === Symbol.for("react.memo_cache_sentinel") ? (k = /* @__PURE__ */ T(ve, {
		asChild: !0,
		children: /* @__PURE__ */ T(h, {
			variant: "outline",
			children: "Menu"
		})
	}), t[12] = k) : k = t[12];
	let A;
	t[13] === Symbol.for("react.memo_cache_sentinel") ? (A = /* @__PURE__ */ E(pe, { children: [k, /* @__PURE__ */ E(me, { children: [
		/* @__PURE__ */ T(ge, { children: "Agent Devices" }),
		/* @__PURE__ */ T(_e, {}),
		/* @__PURE__ */ T(he, { children: "Open" }),
		/* @__PURE__ */ T(he, { children: "Rename" }),
		/* @__PURE__ */ T(he, {
			variant: "destructive",
			children: "Delete"
		})
	] })] }), t[13] = A) : A = t[13];
	let j;
	t[14] === Symbol.for("react.memo_cache_sentinel") ? (j = /* @__PURE__ */ T(fe, {
		asChild: !0,
		children: /* @__PURE__ */ T(h, {
			variant: "outline",
			children: "Dialog"
		})
	}), t[14] = j) : j = t[14];
	let N, I;
	t[15] === Symbol.for("react.memo_cache_sentinel") ? (N = /* @__PURE__ */ E(ue, { children: [/* @__PURE__ */ T(de, { children: "Rename" }), /* @__PURE__ */ T(ce, { children: "What it's called on every device." })] }), I = /* @__PURE__ */ T(xe, { defaultValue: "Anthropic" }), t[15] = N, t[16] = I) : (N = t[15], I = t[16]);
	let L;
	t[17] === Symbol.for("react.memo_cache_sentinel") ? (L = /* @__PURE__ */ E(oe, { children: [j, /* @__PURE__ */ E(se, { children: [
		N,
		I,
		/* @__PURE__ */ T(le, { children: /* @__PURE__ */ T(h, { children: "Save" }) })
	] })] }), t[17] = L) : L = t[17];
	let R;
	t[18] === Symbol.for("react.memo_cache_sentinel") ? (R = /* @__PURE__ */ T(ee, {
		asChild: !0,
		children: /* @__PURE__ */ T(h, {
			variant: "outline",
			children: "Alert dialog"
		})
	}), t[18] = R) : R = t[18];
	let z;
	t[19] === Symbol.for("react.memo_cache_sentinel") ? (z = /* @__PURE__ */ E(d, { children: [/* @__PURE__ */ T(f, { children: "Delete Anthropic?" }), /* @__PURE__ */ T(l, { children: "Apps using it stop working until you add another." })] }), t[19] = z) : z = t[19];
	let B;
	t[20] === Symbol.for("react.memo_cache_sentinel") ? (B = /* @__PURE__ */ E(a, { children: [R, /* @__PURE__ */ E(c, { children: [z, /* @__PURE__ */ E(u, { children: [/* @__PURE__ */ T(s, { children: "Cancel" }), /* @__PURE__ */ T(o, {
		className: "bg-destructive text-white hover:bg-destructive/90",
		children: "Delete"
	})] })] })] }), t[20] = B) : B = t[20];
	let V;
	t[21] === Symbol.for("react.memo_cache_sentinel") ? (V = /* @__PURE__ */ T(Ae, {
		asChild: !0,
		children: /* @__PURE__ */ T(h, {
			variant: "outline",
			children: "Sheet"
		})
	}), t[21] = V) : V = t[21];
	let H;
	t[22] === Symbol.for("react.memo_cache_sentinel") ? (H = /* @__PURE__ */ T(P, {
		title: "Dropdown menu, dialog, alert dialog, sheet",
		children: /* @__PURE__ */ E(F, { children: [
			A,
			L,
			B,
			/* @__PURE__ */ E(Te, { children: [V, /* @__PURE__ */ T(Ee, { children: /* @__PURE__ */ E(Oe, { children: [/* @__PURE__ */ T(ke, { children: "Anthropic" }), /* @__PURE__ */ T(De, { children: "Key · Used by AI and Models" })] }) })] })
		] })
	}), t[22] = H) : H = t[22];
	let U;
	t[23] === Symbol.for("react.memo_cache_sentinel") ? (U = /* @__PURE__ */ T(P, {
		title: "Separator and skeleton",
		children: /* @__PURE__ */ E("div", {
			className: "flex max-w-sm flex-col gap-3",
			children: [
				/* @__PURE__ */ T(je, { className: "h-4 w-3/4" }),
				/* @__PURE__ */ T(je, { className: "h-4 w-1/2" }),
				/* @__PURE__ */ T(we, {}),
				/* @__PURE__ */ T(je, { className: "aspect-[16/10] w-full rounded-xl" })
			]
		})
	}), t[23] = U) : U = t[23];
	let W;
	t[24] === Symbol.for("react.memo_cache_sentinel") ? (W = /* @__PURE__ */ T(Ce, {
		title: "Couldn't reach your devices.",
		text: "Check your connection.",
		action: {
			label: "Try Again",
			run: Ye
		}
	}), t[24] = W) : W = t[24];
	let G;
	t[25] === Symbol.for("react.memo_cache_sentinel") ? (G = /* @__PURE__ */ E(P, {
		title: "Problem: on the thing it's about, with the action that fixes it",
		children: [W, /* @__PURE__ */ T(Ce, {
			warn: !0,
			title: "None of your devices runs Agent Devices.",
			text: "It needs a Mac, or a Linux device with Podman.",
			action: {
				label: "Open App Store",
				run: Je
			}
		})]
	}), t[25] = G) : G = t[25];
	let K;
	t[26] === Symbol.for("react.memo_cache_sentinel") ? (K = /* @__PURE__ */ T(P, {
		title: "Empty: one grey line",
		children: /* @__PURE__ */ T(g, { children: /* @__PURE__ */ T(ye, { children: "None yet. Agents make one when they need a browser." }) })
	}), t[26] = K) : K = t[26];
	let q;
	t[27] === Symbol.for("react.memo_cache_sentinel") ? (q = M.map(qe), t[27] = q) : q = t[27];
	let J;
	t[28] === Symbol.for("react.memo_cache_sentinel") ? (J = /* @__PURE__ */ E(F, { children: [q, /* @__PURE__ */ T(p, { app: {
		name: "x",
		title: "Xylophone"
	} })] }), t[28] = J) : J = t[28];
	let Y;
	t[29] === Symbol.for("react.memo_cache_sentinel") ? (Y = /* @__PURE__ */ E(P, {
		title: "App icons, in each app's colour, and icons by name",
		children: [J, /* @__PURE__ */ T(F, { children: [
			"magnifying-glass",
			"gear",
			"warning-circle-fill",
			"check-circle-fill",
			"browsers"
		].map(Ke) })]
	}), t[29] = Y) : Y = t[29];
	let X;
	t[30] === Symbol.for("react.memo_cache_sentinel") ? (X = /* @__PURE__ */ T(P, {
		title: "Grid: fills the room, at most 4 across here",
		children: /* @__PURE__ */ T(be, {
			min: "12rem",
			max: 4,
			children: M.slice(0, 6).map(Ge)
		})
	}), t[30] = X) : X = t[30];
	let Z;
	t[31] === Symbol.for("react.memo_cache_sentinel") ? (Z = /* @__PURE__ */ T(P, {
		title: "List: rows that stop at a readable width",
		children: /* @__PURE__ */ T(Se, { children: M.slice(0, 4).map(We) })
	}), t[31] = Z) : Z = t[31];
	let Q;
	t[32] === Symbol.for("react.memo_cache_sentinel") ? (Q = /* @__PURE__ */ T(P, {
		title: "Split: what a list opens shows beside it where there's room, else in a sheet",
		children: /* @__PURE__ */ T(ze, {})
	}), t[32] = Q) : Q = t[32];
	let Xe;
	t[33] === Symbol.for("react.memo_cache_sentinel") ? (Xe = { gridTemplateColumns: "minmax(0, 1fr) 22rem" }, t[33] = Xe) : Xe = t[33];
	let Ze;
	t[34] === Symbol.for("react.memo_cache_sentinel") ? (Ze = /* @__PURE__ */ T("div", {
		className: "h-[30rem] overflow-hidden rounded-xl border",
		children: /* @__PURE__ */ T(Be, {})
	}), t[34] = Ze) : Ze = t[34];
	let Qe;
	t[35] === Symbol.for("react.memo_cache_sentinel") ? (Qe = /* @__PURE__ */ T(P, {
		title: "Shell: the window every client shows screens in; where its own room is narrow, the sidebar is a sheet behind the bar's button",
		children: /* @__PURE__ */ E("div", {
			className: "grid gap-4",
			style: Xe,
			children: [Ze, /* @__PURE__ */ T("div", {
				className: "h-[30rem] overflow-hidden rounded-xl border",
				children: /* @__PURE__ */ T(Be, {})
			})]
		})
	}), t[35] = Qe) : Qe = t[35];
	let $;
	return t[36] === Symbol.for("react.memo_cache_sentinel") ? ($ = /* @__PURE__ */ E("main", {
		className: "mx-auto flex flex-col gap-10",
		children: [
			n,
			v,
			y,
			b,
			S,
			C,
			O,
			H,
			U,
			G,
			K,
			Y,
			X,
			Z,
			Q,
			Qe,
			/* @__PURE__ */ T(P, {
				title: "What the window shows over screens when they ask: a menu, an alert, a screen in a sheet",
				children: /* @__PURE__ */ E(F, { children: [
					/* @__PURE__ */ T(h, {
						variant: "outline",
						onClick: Ue,
						children: "Menu"
					}),
					/* @__PURE__ */ T(h, {
						variant: "outline",
						onClick: He,
						children: "Alert"
					}),
					/* @__PURE__ */ T(h, {
						variant: "outline",
						onClick: Ve,
						children: "Sheet"
					})
				] })
			})
		]
	}), t[36] = $) : $ = t[36], $;
}
function Ve() {
	let e = w({ title: "Vault" });
	e.body.innerHTML = "<p style=\"padding:1rem\">A screen goes here, in its frame.</p>";
}
function He() {
	return S({
		title: "Remove Repos?",
		text: "Its data stays until you erase it.",
		action: "Remove",
		danger: !0
	});
}
function Ue(e) {
	let t = e.currentTarget.getBoundingClientRect();
	C([
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
function We(e) {
	return /* @__PURE__ */ T(v, {
		leading: /* @__PURE__ */ T(p, {
			app: e,
			size: 28
		}),
		title: e.title,
		sub: "On every device",
		trailing: e.name === "devices" ? /* @__PURE__ */ T(m, {
			variant: "destructive",
			className: "rounded-full",
			children: "1"
		}) : void 0
	}, e.name);
}
function Ge(e) {
	return /* @__PURE__ */ E("div", {
		className: "min-w-0",
		children: [
			/* @__PURE__ */ T("div", {
				className: "grid aspect-[16/10] place-items-center rounded-xl bg-muted",
				children: /* @__PURE__ */ T(p, {
					app: e,
					size: 48
				})
			}),
			/* @__PURE__ */ T("div", {
				className: "mt-2 truncate text-sm font-medium",
				children: e.title
			}),
			/* @__PURE__ */ T("div", {
				className: "truncate text-sm text-muted-foreground",
				children: "personal"
			})
		]
	}, e.name);
}
function Ke(e) {
	return /* @__PURE__ */ T(_, {
		name: e,
		className: "size-5"
	}, e);
}
function qe(e) {
	return /* @__PURE__ */ T(p, { app: e }, e.name);
}
function Je() {}
function Ye() {}
i(document.getElementById("root")).render(/* @__PURE__ */ T(z, {}));
