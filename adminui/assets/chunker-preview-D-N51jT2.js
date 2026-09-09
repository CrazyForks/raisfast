import{f as c,j as e}from"./vendor-react-j_G0Dyx0.js";import{b8 as u,cr as f,k as j,b3 as y,bT as b}from"./index-CYRNmH_j.js";import{B as t}from"./badge-tlnS4qf3.js";import{C as d,a as o}from"./card-BIikl1fx.js";import{L as k}from"./label-Dyy02eIP.js";import{T as v}from"./textarea-Dn5u6lbX.js";import"./vendor-xyflow-C2hOf6Vp.js";/**
 * @license lucide-react v1.17.0 - ISC
 *
 * This source code is licensed under the ISC license.
 * See the LICENSE file in the root directory of this source tree.
 */const g=[["circle",{cx:"6",cy:"6",r:"3",key:"1lh9wr"}],["path",{d:"M8.12 8.12 12 12",key:"1alkpv"}],["path",{d:"M20 4 8.12 15.88",key:"xgtan2"}],["circle",{cx:"6",cy:"18",r:"3",key:"fqmcym"}],["path",{d:"M14.8 14.8 20 20",key:"ptml3r"}]],N=u("scissors",g);function M(){const{t:a}=f(),[n,m]=c.useState(`# 标题

正文示例…`),[r,x]=c.useState(null),[l,i]=c.useState(!1);async function p(){if(n.trim()){i(!0);try{const s=await y.send("/admin/kb/chunker/preview",{method:"POST",body:{markdown:n}});x(s)}catch(s){b.error(s.message)}finally{i(!1)}}}return e.jsxs("div",{className:"space-y-6",children:[e.jsx(d,{children:e.jsxs(o,{className:"space-y-4 pt-6",children:[e.jsxs("div",{className:"space-y-2",children:[e.jsx(k,{children:a("kb.sample")}),e.jsx(v,{rows:10,value:n,onChange:s=>m(s.target.value),placeholder:`# 章节

内容…

## 小节

内容…`})]}),e.jsx("div",{className:"flex justify-end",children:e.jsxs(j,{size:"sm",disabled:l||!n.trim(),onClick:p,children:[e.jsx(N,{className:"size-4"}),a(l?"kb.running":"kb.runPreview")]})})]})}),r&&e.jsxs("div",{className:"space-y-4",children:[e.jsxs("div",{className:"flex flex-wrap items-center gap-2 text-sm",children:[e.jsx(t,{variant:"outline",children:r.strategy}),e.jsxs(t,{variant:"secondary",children:[a("kb.parents")," ",r.parents]}),e.jsxs(t,{variant:"secondary",children:[a("kb.children")," ",r.children]}),e.jsxs(t,{variant:"secondary",children:[a("kb.total")," ",r.total]}),e.jsxs("span",{className:"text-muted-foreground",children:["parent ",r.parent_size," / child ",r.child_size," 字符"]})]}),e.jsx("div",{className:"space-y-2",children:r.items.map((s,h)=>e.jsx(d,{children:e.jsxs(o,{className:"space-y-1 pt-4 text-sm",children:[e.jsxs("div",{className:"flex flex-wrap items-center gap-2",children:[e.jsx(t,{variant:s.is_child?"secondary":"default",children:s.is_child?a("kb.child"):a("kb.parent")}),e.jsxs("span",{className:"text-muted-foreground",children:[s.bytes," ",a("kb.chars")]}),e.jsxs("span",{className:"text-muted-foreground",children:["[",s.byte_start,", ",s.byte_end,")"]}),s.breadcrumb&&e.jsx("span",{className:"font-mono text-xs text-muted-foreground",children:s.breadcrumb})]}),e.jsx("p",{className:"whitespace-pre-wrap break-all text-muted-foreground",children:s.preview})]})},h))})]})]})}export{M as default};
