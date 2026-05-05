/**
 * raisfast 压力测试 (k6)
 *
 * 用法:
 *     k6 run scripts/benchmark.js                         # 默认 20 VU, 每场景 8s
 *     k6 run scripts/benchmark.js -e VUS=32 -e DUR=10s    # 32 并发, 每场景 10s
 *     k6 run scripts/benchmark.js -e PREPARE=100          # 先插入 100 条测试数据
 *     k6 run scripts/benchmark.js -e ONLY=healthz,posts_list,auth_jwt,auth_token
 *
 * 环境变量:
 *     BASE_URL   服务地址           (默认 http://localhost:9898)
 *     EMAIL      登录邮箱           (默认 admin@test.com)
 *     PASSWORD   登录密码           (默认 Admin1234)
 *     VUS        并发 VU 数         (默认 20)
 *     DUR        每场景持续时间      (默认 8s)
 *     PREPARE    先插入 N 条数据     (默认 0)
 *     ONLY       只跑指定场景        (逗号分隔)
 *     COOLDOWN   场景间隔            (默认 3s)
 */

import http from "k6/http";
import { check } from "k6";
import { Trend } from "k6/metrics";

// ─── 配置 ──────────────────────────────────────────────────────

const BASE = __ENV.BASE_URL || "http://localhost:9898";
const API = `${BASE}/api/v1`;
const EMAIL = __ENV.EMAIL || "admin@test.com";
const PASSWORD = __ENV.PASSWORD || "Admin1234";
const VUS = parseInt(__ENV.VUS || "20", 10);
const DUR_SEC = parseSec(__ENV.DUR || "8s");
const COOLDOWN_SEC = parseInt(__ENV.COOLDOWN || "3", 10);
const PREPARE = parseInt(__ENV.PREPARE || "0", 10);
const ONLY = __ENV.ONLY ? __ENV.ONLY.split(",").map((s) => s.trim()) : null;

// ─── 自定义指标 (按场景名打 tag) ────────────────────────────────

const customMetrics = {};
function trend(name) {
  const t = new Trend(name, true);
  customMetrics[name] = t;
  return t;
}

// ─── 场景定义 ──────────────────────────────────────────────────

const SCENES_DEF = [
  {
    id: "healthz",
    label: "GET /healthz",
    metric: trend("healthz"),
    fn: () => http.get(`${BASE}/healthz`),
    check: (r) => r.status === 200,
  },
  {
    id: "posts_list",
    label: "GET /posts (列表)",
    metric: trend("posts_list"),
    fn: (d) => http.get(`${API}/posts?page=1&page_size=20`, authHdr(d.jwt)),
    check: (r) => r.status === 200,
  },
  {
    id: "cms_list",
    label: "GET /cms/{ct} (列表)",
    metric: trend("cms_list"),
    needs: "cmsPlural",
    fn: (d) => http.get(`${API}/cms/${d.cmsPlural}?page=1&page_size=20`, authHdr(d.jwt)),
    check: (r) => r.status === 200,
  },
  {
    id: "posts_detail",
    label: "GET /posts/{slug}",
    metric: trend("posts_detail"),
    needs: "postSlug",
    fn: (d) => http.get(`${API}/posts/${d.postSlug}`, authHdr(d.jwt)),
    check: (r) => r.status === 200,
  },
  {
    id: "cms_detail",
    label: "GET /cms/{ct}/{id}",
    metric: trend("cms_detail"),
    needs: "cmsItemId",
    fn: (d) => http.get(`${API}/cms/${d.cmsPlural}/${d.cmsItemId}`, authHdr(d.jwt)),
    check: (r) => r.status === 200,
  },
  {
    id: "auth_jwt",
    label: "GET /users/me (JWT)",
    metric: trend("auth_jwt"),
    fn: (d) => http.get(`${API}/users/me`, authHdr(d.jwt)),
    check: (r) => r.status === 200,
  },
  {
    id: "auth_token",
    label: "GET /users/me (API Token)",
    metric: trend("auth_token"),
    fn: (d) => http.get(`${API}/users/me`, authHdr(d.apiToken)),
    check: (r) => r.status === 200,
  },
  {
    id: "auth_token_posts",
    label: "GET /posts (API Token)",
    metric: trend("auth_token_posts"),
    fn: (d) => http.get(`${API}/posts?page=1&page_size=20`, authHdr(d.apiToken)),
    check: (r) => r.status === 200,
  },
  {
    id: "posts_create",
    label: "POST /posts (创建)",
    metric: trend("posts_create"),
    fn: (d, vu, iter) =>
      http.post(
        `${API}/posts`,
        JSON.stringify({
          title: `k6 ${vu}-${iter} ${Date.now()}`,
          content: `Benchmark. ${"z".repeat(200)}`,
          status: "draft",
          category_id: null,
          tag_ids: [],
        }),
        authHdr(d.jwt)
      ),
    check: (r) => r.status === 200,
  },
  {
    id: "cms_create",
    label: "POST /cms/{ct} (创建)",
    metric: trend("cms_create"),
    needs: "cmsPlural",
    fn: (d, vu, iter) =>
      http.post(
        `${API}/cms/${d.cmsPlural}`,
        JSON.stringify({
          title: `k6 ${d.cmsPlural} ${vu}-${iter} ${Date.now()}`,
          content: `Benchmark. ${"z".repeat(200)}`,
          status: "draft",
        }),
        authHdr(d.jwt)
      ),
    check: (r) => r.status === 201 || r.status === 200,
  },
];

// ─── 构建活跃场景列表 ──────────────────────────────────────────

const ACTIVE = SCENES_DEF.filter((s) => {
  if (ONLY && !ONLY.includes(s.id)) return false;
  return true;
});

// ─── k6 options: 每场景一个 constant-vus scenario ─────────────

const scenarios = {};
let offsetMs = 0;
for (const s of ACTIVE) {
  scenarios[s.id] = {
    executor: "constant-vus",
    vus: VUS,
    duration: `${DUR_SEC}s`,
    startTime: `${offsetMs}ms`,
    exec: "benchLoop",
    tags: { scene: s.id },
    env: { _SCENE: s.id },
  };
  offsetMs += (DUR_SEC + COOLDOWN_SEC) * 1000;
}

export const options = {
  scenarios,
  thresholds: {
    http_req_failed: ["rate<0.1"],
    checks: ["rate>0.9"],
  },
};

// ─── Setup ─────────────────────────────────────────────────────

export function setup() {
  const loginRes = http.post(
    `${API}/auth/login`,
    JSON.stringify({ email: EMAIL, password: PASSWORD }),
    { headers: { "Content-Type": "application/json" } }
  );
  check(loginRes, { "login ok": (r) => r.status === 200 });
  const jwt = loginRes.json("data.access_token");
  const h = authHdr(jwt);

  const tokRes = http.post(
    `${API}/tokens`,
    JSON.stringify({ name: "k6-bench", scopes: ["read", "write"] }),
    h
  );
  const apiToken = tokRes.json("data.token");
  check(tokRes, { "api token ok": (r) => r.status === 201 && apiToken });

  let cmsPlural = null;
  let cmsItemId = null;
  const ctRes = http.get(`${API}/admin/content-types`, h);
  if (ctRes.status === 200) {
    for (const ct of ctRes.json("data") || []) {
      if (ct.plural) {
        cmsPlural = ct.plural;
        break;
      }
    }
  }

  let postSlug = null;
  const pRes = http.get(`${API}/posts?page=1&page_size=1`, h);
  if (pRes.status === 200) {
    const items = pRes.json("data.items") || [];
    if (items.length > 0) postSlug = items[0].slug || items[0].id;
  }

  if (cmsPlural) {
    const cRes = http.get(`${API}/cms/${cmsPlural}?page=1&page_size=1`, h);
    if (cRes.status === 200) {
      const items = cRes.json("data.items") || [];
      if (items.length > 0) cmsItemId = items[0].id;
    }
  }

  if (PREPARE > 0) {
    console.log(`准备数据: ${PREPARE} posts + ${cmsPlural ? PREPARE + " cms" : "0 cms"} ...`);
    const batch = {};
    for (let i = 0; i < PREPARE; i++) {
      batch[`p${i}`] = {
        method: "POST",
        url: `${API}/posts`,
        body: JSON.stringify({
          title: `k6-prep-${i}-${Date.now()}`,
          content: `Prep #${i}. ${"x".repeat(200)}`,
          status: "published",
          category_id: null,
          tag_ids: [],
        }),
        headers: h.headers,
      };
    }
    if (cmsPlural) {
      for (let i = 0; i < PREPARE; i++) {
        batch[`c${i}`] = {
          method: "POST",
          url: `${API}/cms/${cmsPlural}`,
          body: JSON.stringify({
            title: `k6-prep-${i}-${Date.now()}`,
            content: `Prep #${i}. ${"y".repeat(200)}`,
            status: "published",
          }),
          headers: h.headers,
        };
      }
    }
    const br = http.batch(batch);
    let ok = 0;
    for (const k in br) if (br[k].status < 300) ok++;
    console.log(`  插入: ${ok}/${Object.keys(br).length} 成功`);
  }

  console.log(`Setup ✓ jwt=${jwt.slice(0, 15)}... apiToken=${apiToken.slice(0, 12)}... cms=${cmsPlural || "none"} postSlug=${postSlug || "none"}`);
  return { jwt, apiToken, cmsPlural, cmsItemId, postSlug };
}

// ─── 主循环 ────────────────────────────────────────────────────

export function benchLoop(data) {
  const sceneId = __ENV._SCENE;
  const scene = ACTIVE.find((s) => s.id === sceneId);
  if (!scene) return;

  if (scene.needs && !data[scene.needs]) return;

  const vu = __VU;
  const iter = __ITER;
  const r = scene.fn(data, vu, iter);
  check(r, { [scene.label]: scene.check });
  scene.metric.add(r.timings.duration);
}

// ─── Summary ───────────────────────────────────────────────────

export function handleSummary(data) {
  const totalReqDur = data.metrics?.http_req_duration?.values;

  let report = "\n";
  report += color("1;36") + "=".repeat(72) + color("0") + "\n";
  report += color("1;36") + "  raisfast Benchmark Report" + color("0") + "\n";
  report += color("1;36") + "=".repeat(72) + color("0") + "\n";
  report += `  VUs: ${VUS}   Duration: ${DUR_SEC}s   Cooldown: ${COOLDOWN_SEC}s   Base: ${BASE}\n\n`;

  // ─── 各场景统计 ─────────────────────────────────────────────
  report += color("1;37") + `  ${"场景".padEnd(38)} ${"Avg".padStart(8)} ${"P50".padStart(8)} ${"P90".padStart(8)} ${"P95".padStart(8)} ${"P99".padStart(8)} ${"Min".padStart(8)} ${"Max".padStart(8)}` + color("0") + "\n";
  report += `  ${"─".repeat(98)}` + "\n";

  const summaries = [];

  for (const s of ACTIVE) {
    const m = data.metrics[s.id];
    if (!m || !m.values || !m.values.avg) {
      report += `  ${s.label.padEnd(38)}    (skipped)\n`;
      continue;
    }

    const v = m.values;
    const info = {
      id: s.id,
      label: s.label,
      avg: v.avg,
      p50: v.med,
      p90: v["p(90)"],
      p95: v["p(95)"],
      min: v.min,
      max: v.max,
    };
    summaries.push(info);

    report += `  ${s.label.padEnd(38)} ${"".padStart(7)} ${ms(v.avg).padStart(8)} ${ms(v.med).padStart(8)} ${ms(v["p(90)"]).padStart(8)} ${ms(v["p(95)"]).padStart(8)} ${ms(v["p(99)"] || 0).padStart(8)} ${ms(v.min).padStart(8)} ${ms(v.max).padStart(8)}\n`;
  }

  report += `  ${"─".repeat(98)}` + "\n\n";

  // ─── 对比汇总 ───────────────────────────────────────────────
  const cmpPairs = [
    ["auth_jwt", "auth_token", "JWT vs Token (/users/me)"],
    ["posts_list", "auth_token_posts", "JWT vs Token (/posts)"],
    ["posts_list", "cms_list", "原生 vs CMS (列表)"],
    ["posts_detail", "cms_detail", "原生 vs CMS (详情)"],
    ["posts_create", "cms_create", "原生 vs CMS (创建)"],
  ];

  const byId = {};
  for (const s of summaries) byId[s.id] = s;

  const hasPairs = cmpPairs.some(([a, b]) => byId[a] && byId[b]);
  if (hasPairs) {
    report += color("1;36") + "=".repeat(72) + color("0") + "\n";
    report += color("1;36") + "  对比汇总" + color("0") + "\n";
    report += color("1;36") + "=".repeat(72) + color("0") + "\n";
    report += `  ${"对比项".padEnd(34)} ${"A Avg".padStart(9)} ${"B Avg".padStart(9)} ${"比值".padStart(7)} ${"A P50".padStart(9)} ${"B P50".padStart(9)} ${"比值".padStart(7)}\n`;
    report += `  ${"─".repeat(88)}` + "\n";

    for (const [aId, bId, label] of cmpPairs) {
      const a = byId[aId];
      const b = byId[bId];
      if (!a || !b) continue;
      const ratioAvg = a.avg > 0 ? b.avg / a.avg : 0;
      const ratioP50 = a.p50 > 0 ? b.p50 / a.p50 : 0;
      report += `  ${label.padEnd(34)} ${ms(a.avg).padStart(9)} ${ms(b.avg).padStart(9)} ${ratioAvg.toFixed(2).padStart(6)}x ${ms(a.p50).padStart(9)} ${ms(b.p50).padStart(9)} ${ratioP50.toFixed(2).padStart(6)}x\n`;
    }
    report += `  ${"─".repeat(88)}` + "\n\n";
  }

  // ─── 全局 ────────────────────────────────────────────────────
  if (totalReqDur) {
    report += `  全局: ${totalReqDur.count || 0} reqs  avg=${ms(totalReqDur.avg || 0)}  p50=${ms(totalReqDur.med || 0)}  p(90)=${ms(totalReqDur["p(90)"] || 0)}  p(95)=${ms(totalReqDur["p(95)"] || 0)}\n`;
  }
  const failRate = data.metrics?.http_req_failed?.values?.rate || 0;
  report += `  失败率: ${(failRate * 100).toFixed(2)}%\n\n`;

  console.log(report);

  return { stdout: report };
}

// ─── 辅助 ──────────────────────────────────────────────────────

function authHdr(token) {
  return { headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" } };
}

function ms(v) {
  return `${(v).toFixed(2)}`;
}

function parseSec(s) {
  const m = s.match(/^(\d+)(s|m)?$/);
  if (!m) return 8;
  const n = parseInt(m[1], 10);
  return m[2] === "m" ? n * 60 : n;
}

function color(code) {
  return __ENV.NO_COLOR ? "" : `\x1b[${code}m`;
}
