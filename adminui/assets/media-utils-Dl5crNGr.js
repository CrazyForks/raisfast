import{a as r}from"./types-C-xO67t8.js";import{a$ as a,a4 as l,X as d}from"./index-C-i99XXF.js";import{A as f}from"./archive-DbNEUzmX.js";import{F as u}from"./file-CwTg0Lel.js";/**
 * @license lucide-react v1.17.0 - ISC
 *
 * This source code is licensed under the ISC license.
 * See the LICENSE file in the root directory of this source tree.
 */const h=[["path",{d:"M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z",key:"1oefj6"}],["path",{d:"M14 2v5a1 1 0 0 0 1 1h5",key:"wfsgrz"}],["path",{d:"M10 12.5 8 15l2 2.5",key:"1tg20x"}],["path",{d:"m14 12.5 2 2.5-2 2.5",key:"yinavb"}]],x=a("file-code",h);/**
 * @license lucide-react v1.17.0 - ISC
 *
 * This source code is licensed under the ISC license.
 * See the LICENSE file in the root directory of this source tree.
 */const y=[["path",{d:"M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z",key:"1oefj6"}],["path",{d:"M14 2v5a1 1 0 0 0 1 1h5",key:"wfsgrz"}],["path",{d:"M11 18h2",key:"12mj7e"}],["path",{d:"M12 12v6",key:"3ahymv"}],["path",{d:"M9 13v-.5a.5.5 0 0 1 .5-.5h5a.5.5 0 0 1 .5.5v.5",key:"qbrxap"}]],g=a("file-type",y);/**
 * @license lucide-react v1.17.0 - ISC
 *
 * This source code is licensed under the ISC license.
 * See the LICENSE file in the root directory of this source tree.
 */const k=[["path",{d:"M9 18V5l12-2v13",key:"1jmyc2"}],["circle",{cx:"6",cy:"18",r:"3",key:"fqmcym"}],["circle",{cx:"18",cy:"16",r:"3",key:"1hluhg"}]],v=a("music",k);/**
 * @license lucide-react v1.17.0 - ISC
 *
 * This source code is licensed under the ISC license.
 * See the LICENSE file in the root directory of this source tree.
 */const M=[["path",{d:"M9 3H5a2 2 0 0 0-2 2v4m6-6h10a2 2 0 0 1 2 2v4M9 3v18m0 0h10a2 2 0 0 0 2-2V9M9 21H5a2 2 0 0 1-2-2V9m0 0h18",key:"gugj83"}]],w=a("table-2",M);/**
 * @license lucide-react v1.17.0 - ISC
 *
 * This source code is licensed under the ISC license.
 * See the LICENSE file in the root directory of this source tree.
 */const _=[["path",{d:"M12 4v16",key:"1654pz"}],["path",{d:"M4 7V5a1 1 0 0 1 1-1h14a1 1 0 0 1 1 1v2",key:"e0r10z"}],["path",{d:"M9 20h6",key:"s66wpe"}]],F=a("type",_);/**
 * @license lucide-react v1.17.0 - ISC
 *
 * This source code is licensed under the ISC license.
 * See the LICENSE file in the root directory of this source tree.
 */const b=[["path",{d:"m16 13 5.223 3.482a.5.5 0 0 0 .777-.416V7.87a.5.5 0 0 0-.752-.432L16 10.5",key:"ftymec"}],["rect",{x:"2",y:"6",width:"14",height:"12",rx:"2",key:"158x01"}]],z=a("video",b),s=[{key:"image",label:"Images",icon:l,mimes:["image/jpeg","image/png","image/gif","image/webp","image/svg+xml","image/bmp","image/avif","image/tiff","image/x-icon","image/heic","image/heif"]},{key:"video",label:"Video",icon:z,mimes:["video/mp4","video/webm","video/quicktime","video/x-matroska"]},{key:"audio",label:"Audio",icon:v,mimes:["audio/mpeg","audio/ogg","audio/wav","audio/aac","audio/flac","audio/opus","audio/mp4"]},{key:"document",label:"Docs",icon:d,mimes:["application/pdf","application/msword","application/vnd.openxmlformats-officedocument.wordprocessingml.document","application/vnd.ms-powerpoint","application/vnd.openxmlformats-officedocument.presentationml.presentation","application/epub+zip","application/rtf"]},{key:"spreadsheet",label:"Sheets",icon:w,mimes:["application/vnd.ms-excel","application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"]},{key:"archive",label:"Archives",icon:f,mimes:["application/zip","application/x-tar","application/gzip","application/x-rar-compressed","application/x-7z-compressed"]},{key:"text",label:"Text",icon:g,mimes:["text/plain","text/csv","text/markdown","application/json","text/html","application/xml","text/yaml","text/x-ini","application/toml","text/css"]},{key:"scripts",label:"Scripts",icon:x,mimes:["application/javascript","text/x-shellscript","application/x-httpd-php","text/x-python","application/sql"]},{key:"font",label:"Fonts",icon:F,mimes:["font/ttf","font/otf","font/woff","font/woff2"]}];function p(e){for(const i of s)if(i.mimes.includes(e))return i.key;return"other"}function C(e){const i=p(e),t=s.find(m=>m.key===i);return(t==null?void 0:t.icon)??u}function I(e,i){return i==="all"?!0:p(e.mimetype)===i}function N(e){if(e==="all"||e==="other")return"";const i=s.find(t=>t.key===e);return(i==null?void 0:i.mimes.join(","))??""}const V=r;function E(e){return V[e]??e}function q(e){return e<1024?`${e} B`:e<1024*1024?`${(e/1024).toFixed(1)} KB`:e<1024*1024*1024?`${(e/(1024*1024)).toFixed(1)} MB`:`${(e/(1024*1024*1024)).toFixed(1)} GB`}function B(e){return e.startsWith("image/")}function L(e){return e.startsWith("video/")}function S(e){return e.startsWith("audio/")}function D(e){return e==="application/pdf"}function W(e,i,t){return[...e].sort((n,c)=>{let o;switch(i){case"filename":o=n.filename.localeCompare(c.filename);break;case"size":o=n.size-c.size;break;case"created_at":default:o=new Date(n.created_at).getTime()-new Date(c.created_at).getTime();break}return t==="desc"?-o:o})}export{s as F,w as T,z as V,x as a,F as b,p as c,C as d,B as e,q as f,N as g,D as h,S as i,L as j,E as k,I as m,W as s};
