import{a$ as c,Z as d,N as p}from"./index-C8CQ70BB.js";import{A as l}from"./archive-Cg51PBNJ.js";import{F as u}from"./file-C2MLXbcx.js";/**
 * @license lucide-react v1.17.0 - ISC
 *
 * This source code is licensed under the ISC license.
 * See the LICENSE file in the root directory of this source tree.
 */const f=[["path",{d:"M9 18V5l12-2v13",key:"1jmyc2"}],["circle",{cx:"6",cy:"18",r:"3",key:"fqmcym"}],["circle",{cx:"18",cy:"16",r:"3",key:"1hluhg"}]],g=c("music",f);/**
 * @license lucide-react v1.17.0 - ISC
 *
 * This source code is licensed under the ISC license.
 * See the LICENSE file in the root directory of this source tree.
 */const h=[["path",{d:"M9 3H5a2 2 0 0 0-2 2v4m6-6h10a2 2 0 0 1 2 2v4M9 3v18m0 0h10a2 2 0 0 0 2-2V9M9 21H5a2 2 0 0 1-2-2V9m0 0h18",key:"gugj83"}]],v=c("table-2",h);/**
 * @license lucide-react v1.17.0 - ISC
 *
 * This source code is licensed under the ISC license.
 * See the LICENSE file in the root directory of this source tree.
 */const k=[["path",{d:"m16 13 5.223 3.482a.5.5 0 0 0 .777-.416V7.87a.5.5 0 0 0-.752-.432L16 10.5",key:"ftymec"}],["rect",{x:"2",y:"6",width:"14",height:"12",rx:"2",key:"158x01"}]],x=c("video",k),r=[{key:"image",label:"Images",icon:d,mimes:["image/jpeg","image/png","image/gif","image/webp","image/svg+xml"]},{key:"video",label:"Video",icon:x,mimes:["video/mp4","video/webm","video/quicktime"]},{key:"audio",label:"Audio",icon:g,mimes:["audio/mpeg","audio/ogg","audio/wav","audio/aac"]},{key:"document",label:"Docs",icon:p,mimes:["application/pdf","application/msword","application/vnd.openxmlformats-officedocument.wordprocessingml.document","application/vnd.ms-powerpoint","application/vnd.openxmlformats-officedocument.presentationml.presentation"]},{key:"spreadsheet",label:"Sheets",icon:v,mimes:["application/vnd.ms-excel","application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"]},{key:"archive",label:"Archives",icon:l,mimes:["application/zip","application/x-tar","application/gzip","application/x-rar-compressed"]}];function s(e){for(const i of r)if(i.mimes.includes(e))return i.key;return"other"}function w(e){const i=s(e),t=r.find(m=>m.key===i);return(t==null?void 0:t.icon)??u}function _(e,i){return i==="all"?!0:s(e.mimetype)===i}function V(e){if(e==="all"||e==="other")return"";const i=r.find(t=>t.key===e);return(i==null?void 0:i.mimes.join(","))??""}function A(e){return e<1024?`${e} B`:e<1024*1024?`${(e/1024).toFixed(1)} KB`:e<1024*1024*1024?`${(e/(1024*1024)).toFixed(1)} MB`:`${(e/(1024*1024*1024)).toFixed(1)} GB`}function I(e){return e.startsWith("image/")}function $(e){return e.startsWith("video/")}function b(e){return e.startsWith("audio/")}function z(e){return e==="application/pdf"}function C(e,i,t){return[...e].sort((o,n)=>{let a;switch(i){case"filename":a=o.filename.localeCompare(n.filename);break;case"size":a=o.size-n.size;break;case"created_at":default:a=new Date(o.created_at).getTime()-new Date(n.created_at).getTime();break}return t==="desc"?-a:a})}export{r as F,x as V,w as a,I as b,z as c,$ as d,A as f,V as g,b as i,_ as m,C as s};
