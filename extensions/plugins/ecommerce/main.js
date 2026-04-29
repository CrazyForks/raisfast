import { dbQuery, dbExec, ok, fail, extractJson, newId, dbBegin, dbCommit, dbRollback } from 'sdk';

const nowISO = () => new Date().toISOString();

// ── GET /products ────────────────────────────────────────────

export function listProducts() {
    const rows = dbQuery("SELECT id, name, slug, price, compare_at_price, stock, sku, images, featured FROM products WHERE status = 'published' ORDER BY created_at DESC LIMIT 50");
    if (!rows) return fail(500, "query failed");
    return ok({ items: rows, total: rows.length });
}

// ── GET /products/:id ────────────────────────────────────────

export function getProduct(input) {
    const id = extractJson(input, "params.id");
    const rows = dbQuery("SELECT id, name, slug, description, price, compare_at_price, stock, sku, images, featured, weight FROM products WHERE id = ? AND status = 'published'", [id]);
    if (!rows) return fail(500, "query failed");
    if (rows.length === 0) return fail(404, "product not found");
    return ok(rows[0]);
}

// ── GET /cart ────────────────────────────────────────────────

export function viewCart(input) {
    const data = extractJson(input, "body");
    const userId = data.user_id;
    if (!userId) return fail(400, "user_id required");
    const rows = dbQuery("SELECT c.id as cart_id, c.product_id, c.quantity, p.name, p.price, p.stock, p.sku FROM cart_items c LEFT JOIN products p ON c.product_id = p.id WHERE c.user_id = ?", [userId]);
    if (!rows) return fail(500, "query failed");
    let total = 0;
    for (const row of rows) {
        row.subtotal = (row.price || 0) * (row.quantity || 0);
        total += row.subtotal;
    }
    return ok({ items: rows, total });
}

// ── POST /cart ───────────────────────────────────────────────

export function addToCart(input) {
    const data = extractJson(input, "body");
    const userId = data.user_id;
    const productId = data.product_id;
    const quantity = data.quantity || 1;
    if (!userId || !productId) return fail(400, "user_id and product_id required");

    const products = dbQuery("SELECT id, name, price, stock, status FROM products WHERE id = ?", [productId]);
    if (!products) return fail(500, "query failed");
    if (products.length === 0) return fail(404, "product not found");
    if (products[0].status !== "published") return fail(400, "product not available");
    if (products[0].stock < quantity) return fail(400, "insufficient stock");

    const existing = dbQuery("SELECT id, quantity FROM cart_items WHERE user_id = ? AND product_id = ?", [userId, productId]);
    if (!existing) return fail(500, "query failed");

    if (existing.length > 0) {
        const newQty = existing[0].quantity + quantity;
        const r = dbExec("UPDATE cart_items SET quantity = ? WHERE id = ?", [newQty, existing[0].id]);
        if (r.error) return fail(500, r.error);
    } else {
        const id = newId();
        const now = nowISO();
        const r = dbExec("INSERT INTO cart_items (id, tenant_id, user_id, product_id, quantity, created_at, updated_at) VALUES (?, 'default', ?, ?, ?, ?, ?)", [id, userId, productId, quantity, now, now]);
        if (r.error) return fail(500, r.error);
    }
    return ok({ added: true });
}

// ── DELETE /cart ─────────────────────────────────────────────

export function clearCart(input) {
    const data = extractJson(input, "body");
    if (!data.user_id) return fail(400, "user_id required");
    const r = dbExec("DELETE FROM cart_items WHERE user_id = ?", [data.user_id]);
    if (r.error) return fail(500, r.error);
    return ok({ cleared: true, rows_affected: r.rows_affected });
}

// ── PUT /cart/:id ────────────────────────────────────────────

export function updateCartItem(input) {
    const itemId = extractJson(input, "params.id");
    const data = extractJson(input, "body");
    if (!itemId) return fail(400, "item id required");
    if (!data.quantity || data.quantity < 1) return fail(400, "quantity must be >= 1");
    const r = dbExec("UPDATE cart_items SET quantity = ? WHERE id = ?", [data.quantity, itemId]);
    if (r.error) return fail(500, r.error);
    if (r.rows_affected === 0) return fail(404, "cart item not found");
    return ok({ updated: true });
}

// ── DELETE /cart/:id ─────────────────────────────────────────

export function removeCartItem(input) {
    const itemId = extractJson(input, "params.id");
    if (!itemId) return fail(400, "item id required");
    const r = dbExec("DELETE FROM cart_items WHERE id = ?", [itemId]);
    if (r.error) return fail(500, r.error);
    if (r.rows_affected === 0) return fail(404, "cart item not found");
    return ok({ removed: true });
}

// ── POST /checkout (事务) ────────────────────────────────────

export function checkout(input) {
    const data = extractJson(input, "body");
    const userId = data.user_id;
    if (!userId) return fail(400, "user_id required");

    const cartItems = dbQuery("SELECT c.id as cart_id, c.product_id, c.quantity, p.name, p.price, p.stock, p.status FROM cart_items c LEFT JOIN products p ON c.product_id = p.id WHERE c.user_id = ?", [userId]);
    if (!cartItems) return fail(500, "query failed");
    if (cartItems.length === 0) return fail(400, "cart is empty");

    for (const item of cartItems) {
        if (!item.name) return fail(400, `product ${item.product_id} not found`);
        if (item.status !== "published") return fail(400, `product ${item.name} not available`);
        if (item.stock < item.quantity) return fail(400, `insufficient stock for ${item.name}`);
    }

    let totalAmount = 0;
    const orderItems = [];
    for (const item of cartItems) {
        const subtotal = item.price * item.quantity;
        totalAmount += subtotal;
        orderItems.push({ product_id: item.product_id, product_name: item.name, price: item.price, quantity: item.quantity, subtotal });
    }

    dbBegin();

    const orderId = newId();
    const orderNo = `ORD-${Date.now().toString(36).toUpperCase()}-${Math.random().toString(36).substring(2, 6).toUpperCase()}`;
    const shippingJson = data.shipping_address ? JSON.stringify(data.shipping_address) : "";

    const r = dbExec(
        "INSERT INTO orders (id, tenant_id, order_no, user_id, status, total_amount, shipping_address, note, created_at, updated_at) VALUES (?, 'default', ?, ?, 'pending', ?, ?, ?, ?, ?)",
        [orderId, orderNo, userId, totalAmount, shippingJson, data.note || "", nowISO(), nowISO()]
    );
    if (r.error) {
        dbRollback();
        return fail(500, `create order failed: ${r.error}`);
    }

    for (const oi of orderItems) {
        const oiId = newId();
        const r2 = dbExec(
            "INSERT INTO order_items (id, order_id, product_id, product_name, price, quantity, subtotal) VALUES (?, ?, ?, ?, ?, ?, ?)",
            [oiId, orderId, oi.product_id, oi.product_name, oi.price, oi.quantity, oi.subtotal]
        );
        if (r2.error) {
            dbRollback();
            return fail(500, `create order item failed: ${r2.error}`);
        }

        const r3 = dbExec("UPDATE products SET stock = stock - ? WHERE id = ? AND stock >= ?", [oi.quantity, oi.product_id, oi.quantity]);
        if (r3.error || r3.rows_affected === 0) {
            dbRollback();
            return fail(500, `stock deduction failed for ${oi.product_name}`);
        }
    }

    dbExec("DELETE FROM cart_items WHERE user_id = ?", [userId]);

    dbCommit();

    return ok({
        order_id: orderId,
        order_no: orderNo,
        status: "pending",
        total_amount: totalAmount,
        items_count: orderItems.length,
    });
}

// ── GET /orders ──────────────────────────────────────────────

export function listOrders(input) {
    const data = extractJson(input, "body");
    if (!data.user_id) return fail(400, "user_id required");
    const rows = dbQuery("SELECT id, order_no, status, total_amount, note, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC LIMIT 50", [data.user_id]);
    if (!rows) return fail(500, "query failed");
    return ok({ items: rows, total: rows.length });
}

// ── GET /orders/:id ──────────────────────────────────────────

export function getOrder(input) {
    const orderId = extractJson(input, "params.id");
    const data = extractJson(input, "body");
    if (!orderId) return fail(400, "order id required");
    const orders = dbQuery("SELECT id, order_no, user_id, status, total_amount, shipping_address, note, paid_at, shipped_at, created_at FROM orders WHERE id = ?", [orderId]);
    if (!orders) return fail(500, "query failed");
    if (orders.length === 0) return fail(404, "order not found");
    const order = orders[0];
    if (data.user_id && order.user_id !== data.user_id) return fail(403, "forbidden");

    const items = dbQuery("SELECT id, product_id, product_name, price, quantity, subtotal FROM order_items WHERE order_id = ?", [orderId]);
    order.items = items || [];
    return ok(order);
}
