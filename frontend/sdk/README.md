# @raisfast/sdk

Framework-agnostic JavaScript/TypeScript SDK for [RaisFast](https://github.com/anomalyco/raisfast).

## Install

```bash
pnpm add @raisfast/sdk
# or
npm install @raisfast/sdk
```

## Quick Start

```ts
import { RaisFast } from "@raisfast/sdk";

const client = new RaisFast("http://localhost:9000/api/v1");

// Auth
await client.auth.login("user@example.com", "password");
console.log(client.auth.user);

// Collection CRUD
const posts = client.collection<Post>("posts");
const list = await posts.getList(1, 25);
const post = await posts.getOne("slug-or-id");
const created = await posts.create({ title: "Hello", body: "World" });
await posts.update(created.id, { title: "Updated" });
await posts.delete(created.id);
```

## Auth

```ts
// Login (auto-saves to localStorage)
await client.auth.login("user@example.com", "password");

// Register
await client.auth.register({
  email: "user@example.com",
  password: "secret",
  nickname: "Alice",
});

// Get current user
const me = await client.auth.getMe();

// Update profile
await client.auth.updateMe({ nickname: "New Name" });

// Change password
await client.auth.changePassword({ old_password: "old", new_password: "new" });

// Logout
await client.auth.logout();
```

The default `LocalAuthStore` persists auth state to `localStorage`. Access token refresh is handled automatically on 401 responses.

### Custom Auth Store

```ts
import { BaseAuthStore } from "@raisfast/sdk";

class MyStore extends BaseAuthStore {
  // override save/clear to sync with your state management
}

const client = new RaisFast("http://localhost:9000/api/v1", {
  authStore: new MyStore(),
});
```

### Listen for Auth Changes

```ts
const unsub = client.authStore.onChange((token, user) => {
  console.log("auth changed:", { token, user });
}, true); // fire immediately with current state

unsub(); // unsubscribe
```

## Collection

```ts
const posts = client.collection<Post>("posts");

// Paginated list
const page = await posts.getList(1, 25, {
  sort: "-created_at",
  filter: 'status = "published"',
  search: "hello",
  fields: "id,title",
});

// Full list (auto-paginates)
const all = await posts.getFullList({ sort: "title" });

// First item matching filter
const first = await posts.getFirstListItem('slug = "hello-world"');

// Single item
const post = await posts.getOne("id-or-slug");

// Create / Update / Delete
const created = await posts.create({ title: "New Post" });
await posts.update(created.id, { title: "Updated" });
await posts.delete(created.id);
```

### Admin Collection

Admin collections use the `/admin/cms/` prefix:

```ts
const adminPosts = client.adminCollection<Post>("posts");
```

## Admin

```ts
// Dashboard stats
const stats = await client.admin.stats();

// Content stats
const contentStats = await client.admin.statsContent("posts");

// Trends
const trends = await client.admin.statsTrends("posts", 30);

// Plugins
const plugins = await client.admin.listPlugins();
await client.admin.enablePlugin("my-plugin");

// Content Types
const types = await client.admin.listContentTypes();
await client.admin.createContentType({ name: "posts", ... });
await client.admin.deleteContentType("posts");
```

## Request Hooks

### beforeSend

Intercept requests before they are sent (e.g. add custom headers):

```ts
client.beforeSend = (url, options) => {
  console.log("Requesting:", url);
  return { url, options };
};
```

### afterSend

Transform responses after they are received:

```ts
client.afterSend = (response, data) => {
  console.log("Response status:", response.status);
  return data;
};
```

## Request Options

All request methods accept an optional `RequestOptions`:

```ts
await posts.getList(1, 25, {
  headers: { "X-Custom": "value" },
  query: { foo: "bar" },
  signal: abortController.signal,
  fetch: customFetch, // override global fetch
});
```

## Multi-tenant

```ts
client.setTenantId("tenant-123");
client.setTenantId(null); // reset to default
```

## Error Handling

```ts
import { SDKError } from "@raisfast/sdk";

try {
  await posts.getOne("nonexistent");
} catch (e) {
  if (e instanceof SDKError) {
    console.log(e.code);        // backend error code
    console.log(e.status);      // HTTP status
    console.log(e.message);     // error message
    console.log(e.url);         // request URL
    console.log(e.response);    // full response body
    console.log(e.isAbort);     // was request aborted
    console.log(e.originalError); // original Error if any
  }
}
```

## TypeScript

The SDK is written in TypeScript and ships type definitions. Generic parameters are available for collections:

```ts
interface Post {
  id: string;
  title: string;
  body: string;
  created_at: string;
}

const posts = client.collection<Post>("posts");
const post = await posts.getOne("slug"); // typed as Post
```

## Build

```bash
pnpm build
```

Outputs CJS + ESM + type declarations via [tsup](https://tsup.egoist.dev/).

## License

MIT
