#!/usr/bin/env python3
"""
raisfast CMS 动态路由 vs 原生路由 压力测试

用法:
    python3 scripts/benchmark.py                     # 默认参数
    python3 scripts/benchmark.py --concurrency 50    # 50 并发
    python3 scripts/benchmark.py --duration 10       # 持续 10 秒
    python3 scripts/benchmark.py --prepare 100       # 先插入 100 条测试数据
"""

import argparse
import asyncio
import json
import random
import string
import time
from dataclasses import dataclass, field

import httpx

BASE_URL = "http://localhost:9898/api/v1"
AUTH = {"email": "admin@test.com", "password": "Admin1234"}


@dataclass
class Stats:
    name: str = ""
    total: int = 0
    success: int = 0
    errors: int = 0
    latencies: list[float] = field(default_factory=list)
    status_codes: dict[int, int] = field(default_factory=dict)

    def record(self, latency: float, status: int):
        self.total += 1
        if 200 <= status < 300:
            self.success += 1
        else:
            self.errors += 1
        self.latencies.append(latency)
        self.status_codes[status] = self.status_codes.get(status, 0) + 1

    def report(self) -> str:
        if not self.latencies:
            return f"{self.name}: no data"
        lats = sorted(self.latencies)
        n = len(lats)
        avg = sum(lats) / n
        p50 = lats[int(n * 0.50)]
        p90 = lats[int(n * 0.90)]
        p95 = lats[int(n * 0.95)]
        p99 = lats[min(int(n * 0.99), n - 1)]
        rps = n / sum(lats) if sum(lats) > 0 else 0
        lines = [
            f"{'='*60}",
            f"  {self.name}",
            f"{'='*60}",
            f"  Requests:    {n} ({self.errors} errors)",
            f"  RPS:         {rps:.1f} req/s",
            f"  Avg:         {avg*1000:.2f} ms",
            f"  P50:         {p50*1000:.2f} ms",
            f"  P90:         {p90*1000:.2f} ms",
            f"  P95:         {p95*1000:.2f} ms",
            f"  P99:         {p99*1000:.2f} ms",
            f"  Min:         {lats[0]*1000:.2f} ms",
            f"  Max:         {lats[-1]*1000:.2f} ms",
        ]
        if self.status_codes:
            codes = ", ".join(f"{k}={v}" for k, v in sorted(self.status_codes.items()))
            lines.append(f"  Status:      {codes}")
        return "\n".join(lines)


async def get_token(client: httpx.AsyncClient) -> str:
    resp = await client.post(f"{BASE_URL}/auth/login", json=AUTH)
    data = resp.json()
    return data["data"]["access_token"]


async def fetch_content_types(client: httpx.AsyncClient, token: str) -> list[dict]:
    """从 /admin/content-types 获取已注册的 CMS content type 列表"""
    headers = make_headers(token)
    resp = await client.get(f"{BASE_URL}/admin/content-types", headers=headers)
    data = resp.json()
    items = data.get("data", [])
    return [ct for ct in items if ct.get("plural")]


async def pick_cms_plural(
    client: httpx.AsyncClient, token: str, prefer: str | None = None
) -> str | None:
    """选一个可用的 CMS plural 名称，优先匹配 prefer"""
    content_types = await fetch_content_types(client, token)
    if not content_types:
        return None
    if prefer:
        for ct in content_types:
            if ct["plural"] == prefer:
                return ct["plural"]
    return content_types[0]["plural"]


async def prepare_data(
    client: httpx.AsyncClient, token: str, count: int, cms_plural: str | None
):
    """插入测试数据（原生 posts + CMS）"""
    headers = {"Authorization": f"Bearer {token}"}
    suffix = "".join(random.choices(string.ascii_lowercase, k=6))

    print(f"\n准备测试数据: {count} 条 posts", end="")
    if cms_plural:
        print(f" + {count} 条 {cms_plural}", end="")
    print(" ...")

    async def create_post(i: int):
        payload = {
            "title": f"Bench Post {suffix} {i}",
            "content": f"Benchmark content #{i}. " + "x" * 200,
            "status": "published",
            "category_id": None,
            "tag_ids": [],
        }
        resp = await client.post(f"{BASE_URL}/posts", json=payload, headers=headers)
        return resp

    async def create_cms(i: int):
        payload = {
            "title": f"Bench {cms_plural} {suffix} {i}",
            "content": f"Benchmark #{i}. " + "y" * 200,
            "status": "published",
        }
        resp = await client.post(
            f"{BASE_URL}/cms/{cms_plural}", json=payload, headers=headers
        )
        return resp

    tasks = []
    for i in range(count):
        tasks.append(create_post(i))
    if cms_plural:
        for i in range(count):
            tasks.append(create_cms(i))
    responses = await asyncio.gather(*tasks)
    ok = sum(1 for r in responses if r.status_code < 300)
    print(f"  插入完成: {ok}/{len(responses)} 成功")


async def run_scenario(
    client: httpx.AsyncClient,
    token: str,
    url: str,
    concurrency: int,
    duration: float,
    method: str = "GET",
    payload: dict | None = None,
    payload_fn: object | None = None,
    params: dict | None = None,
) -> Stats:
    headers = make_headers(token)
    stats = Stats(name=f"{method} {url}")
    stop_time = time.monotonic() + duration
    sem = asyncio.Semaphore(concurrency)
    counter = 0

    async def worker():
        nonlocal counter
        while time.monotonic() < stop_time:
            async with sem:
                counter += 1
                req_payload = payload_fn(counter) if payload_fn else payload
                start = time.monotonic()
                try:
                    if method == "GET":
                        resp = await client.get(url, headers=headers, params=params)
                    else:
                        resp = await client.post(url, json=req_payload, headers=headers)
                    latency = time.monotonic() - start
                    stats.record(latency, resp.status_code)
                except Exception as e:
                    latency = time.monotonic() - start
                    stats.record(latency, 0)

    workers = [asyncio.create_task(worker()) for _ in range(concurrency)]
    await asyncio.gather(*workers)
    return stats


def make_headers(token: str | None) -> dict:
    return {"Authorization": f"Bearer {token}"} if token else {}


async def fetch_first_id(client: httpx.AsyncClient, token: str, url: str) -> str:
    """从列表 API 获取第一条记录的 ID"""
    for attempt in range(10):
        await asyncio.sleep(0.5 * (attempt + 1))
        resp = await client.get(url, headers=make_headers(token))
        data = resp.json()
        if resp.status_code == 429:
            continue
        items = data.get("data", {}).get("items", [])
        if items:
            return items[0]["id"]
    raise RuntimeError(f"无法从 {url} 获取记录 ID")


async def main():
    parser = argparse.ArgumentParser(description="raisfast 压力测试")
    parser.add_argument("--concurrency", "-c", type=int, default=20, help="并发数")
    parser.add_argument("--duration", "-d", type=float, default=5, help="持续时间（秒）")
    parser.add_argument("--prepare", "-p", type=int, default=0, help="先插入 N 条测试数据")
    parser.add_argument("--url", type=str, default=None, help="自定义测试 URL")
    args = parser.parse_args()

    print(f"raisfast 压力测试  |  并发={args.concurrency}  持续={args.duration}s")
    print(f"目标: {BASE_URL}")

    async with httpx.AsyncClient(
        timeout=30,
        limits=httpx.Limits(max_connections=1000, max_keepalive_connections=500),
    ) as client:
        # 获取 token
        print("\n获取认证 token ...")
        token = await get_token(client)
        print(f"  Token: {token[:20]}...")

        # 探测已注册的 CMS content type
        cms_plural = await pick_cms_plural(client, token, prefer="articles")
        if cms_plural:
            print(f"  CMS content type: {cms_plural}")
        else:
            print("  CMS: 无已注册 content type，跳过 CMS 场景")

        # 准备数据
        if args.prepare > 0:
            await prepare_data(client, token, args.prepare, cms_plural)

        # 自定义 URL 模式
        if args.url:
            stats = await run_scenario(client, token, args.url, args.concurrency, args.duration)
            print(stats.report())
            return

        results: list[Stats] = []

        # ── 场景 1: 原生列表 vs CMS 列表（只读）───────────────────────
        print(f"\n[1/6] 原生 GET /posts (列表)")
        s1 = await run_scenario(
            client, token, f"{BASE_URL}/posts",
            args.concurrency, args.duration,
            params={"page": "1", "page_size": "20"},
        )
        results.append(s1)
        print(f"  完成: {s1.total} 请求, {s1.errors} 错误")
        await asyncio.sleep(5)

        if cms_plural:
            print(f"[2/6] CMS    GET /cms/{cms_plural} (列表)")
            s2 = await run_scenario(
                client, token, f"{BASE_URL}/cms/{cms_plural}",
                args.concurrency, args.duration,
                params={"page": "1", "page_size": "20"},
            )
            results.append(s2)
            print(f"  完成: {s2.total} 请求, {s2.errors} 错误")
        else:
            print("[2/6] CMS    跳过（无 content type）")
            results.append(Stats(name=f"GET /cms/?"))
        await asyncio.sleep(5)

        # ── 场景 2: 原生详情 vs CMS 详情 ──────────────────────────────
        resp = await client.get(f"{BASE_URL}/posts?page=1&page_size=1", headers=make_headers(token))
        post_data = resp.json().get("data")
        if not post_data or not post_data.get("items"):
            print("  跳过: 原生 /posts 无数据，跳过详情和创建场景")
            for label in ["原生 GET /posts/{slug}", f"CMS GET /cms/{cms_plural}/{{id}}" if cms_plural else "CMS skip",
                          "原生 POST /posts", f"CMS POST /cms/{cms_plural}" if cms_plural else "CMS skip"]:
                results.append(Stats(name=label))
            for s in results:
                print(s.report())
            return
        post_item = post_data["items"][0]
        post_id = post_item["id"]
        post_slug = post_item.get("slug", post_id)

        print(f"[3/6] 原生 GET /posts/{{slug}} (详情)")
        results.append(
            await run_scenario(
                client, token, f"{BASE_URL}/posts/{post_slug}",
                args.concurrency, args.duration,
            )
        )
        await asyncio.sleep(5)

        if cms_plural:
            article_id = await fetch_first_id(client, token, f"{BASE_URL}/cms/{cms_plural}?page=1&page_size=1")
            print(f"[4/6] CMS    GET /cms/{cms_plural}/{{id}} (详情)")
            s4 = await run_scenario(
                client, token, f"{BASE_URL}/cms/{cms_plural}/{article_id}",
                args.concurrency, args.duration,
            )
            results.append(s4)
            print(f"  完成: {s4.total} 请求, {s4.errors} 错误")
        else:
            print("[4/6] CMS    跳过（无 content type）")
            results.append(Stats(name=f"GET /cms/?"))
        await asyncio.sleep(5)

        # ── 场景 3: API Token 认证 vs JWT 认证（读取列表）────────────
        print("[5/8] JWT    GET /posts (列表, JWT auth)")
        s5_jwt = await run_scenario(
            client, token, f"{BASE_URL}/posts",
            args.concurrency, args.duration,
            params={"page": "1", "page_size": "20"},
        )
        results.append(s5_jwt)
        print(f"  完成: {s5_jwt.total} 请求, {s5_jwt.errors} 错误")
        await asyncio.sleep(5)

        api_tok_resp = await client.post(
            f"{BASE_URL}/tokens",
            json={"name": "bench-token", "scopes": ["read"]},
            headers=make_headers(token),
        )
        api_tok = api_tok_resp.json()["data"]["token"]
        print(f"[6/8] Token  GET /posts (列表, API Token auth)")
        s5_at = await run_scenario(
            client, api_tok, f"{BASE_URL}/posts",
            args.concurrency, args.duration,
            params={"page": "1", "page_size": "20"},
        )
        results.append(s5_at)
        print(f"  完成: {s5_at.total} 请求, {s5_at.errors} 错误")
        await asyncio.sleep(5)

        # ── 场景 4: 写入性能 ─────────────────────────────────────────
        suffix = "".join(random.choices(string.ascii_lowercase, k=6))

        print(f"[7/10] 原生 POST /posts (创建)")
        results.append(
            await run_scenario(
                client, token, f"{BASE_URL}/posts",
                args.concurrency, args.duration,
                method="POST",
                payload_fn=lambda i: {
                    "title": f"Bench Post {suffix} {i} {time.monotonic_ns()}",
                    "content": "Benchmark post content. " + "z" * 200,
                    "status": "draft",
                    "category_id": None,
                    "tag_ids": [],
                },
            )
        )
        await asyncio.sleep(5)

        if cms_plural:
            print(f"[8/10] CMS    POST /cms/{cms_plural} (创建)")
            captured_plural = cms_plural
            results.append(
                await run_scenario(
                    client, token, f"{BASE_URL}/cms/{captured_plural}",
                    args.concurrency, args.duration,
                    method="POST",
                    payload_fn=lambda i: {
                        "title": f"Bench {captured_plural} {suffix} {i} {time.monotonic_ns()}",
                        "content": "Benchmark content. " + "z" * 200,
                        "status": "draft",
                    },
                )
            )
        else:
            print("[8/10] CMS    跳过（无 content type）")
            results.append(Stats(name=f"POST /cms/?"))

        # ── 场景 5: healthz (无认证, 纯 HTTP 开销) ──────────────────
        print("[9/10] GET /healthz (无认证)")
        s9 = await run_scenario(
            client, None, f"{BASE_URL.replace('/api/v1', '')}/healthz",
            args.concurrency, args.duration,
        )
        results.append(s9)
        print(f"  完成: {s9.total} 请求, {s9.errors} 错误")
        await asyncio.sleep(3)

        # ── 场景 6: API Token 连续请求 (测缓存热路径) ───────────────
        print("[10/10] Token GET /users/me (API Token 热路径)")
        s10 = await run_scenario(
            client, api_tok, f"{BASE_URL}/users/me",
            args.concurrency, args.duration,
        )
        results.append(s10)
        print(f"  完成: {s10.total} 请求, {s10.errors} 错误")

    # ── 输出报告 ──────────────────────────────────────────────────────
    print("\n")
    for s in results:
        print(s.report())
        print()

    # ── 对比汇总 ──────────────────────────────────────────────────────
    if len(results) >= 10 and cms_plural:
        print("=" * 60)
        print("  对比汇总")
        print("=" * 60)

        pairs = [
            (results[0], results[1], "列表读取"),
            (results[2], results[3], "详情读取"),
            (results[4], results[5], "JWT vs Token 认证"),
            (results[6], results[7], "记录创建"),
        ]

        for native, cms, label in pairs:
            if not native.latencies or not cms.latencies:
                continue
            n_avg = sum(native.latencies) / len(native.latencies) * 1000
            c_avg = sum(cms.latencies) / len(cms.latencies) * 1000
            ratio = c_avg / n_avg if n_avg > 0 else 0
            print(f"  {label:8s}  原生={n_avg:7.2f}ms  CMS={c_avg:7.2f}ms  比值={ratio:.2f}x")

        print("=" * 60)


if __name__ == "__main__":
    asyncio.run(main())
