var Plugin = {};

function ok(result) {
    if (result && result._error) {
        return JSON.stringify({ status: result._status || 400, body: JSON.stringify({ ok: false, error: result._error }) });
    }
    return JSON.stringify({ status: 200, body: JSON.stringify({ ok: true, data: result }) });
}

function err(status, msg) {
    return { _error: msg, _status: status };
}

function parseBody(input) {
    try {
        if (typeof input === "string") {
            var parsed = JSON.parse(input);
            if (parsed && typeof parsed.body === "string" && parsed.body.charAt(0) === "{") {
                return JSON.parse(parsed.body);
            }
            return parsed;
        }
        if (input && input.body) return JSON.parse(input.body);
        return {};
    } catch (e) { return {}; }
}

function routeParam(input, index) {
    var obj = input;
    if (typeof input === "string") {
        try { obj = JSON.parse(input); } catch (e) { return ""; }
    }
    var path = (obj.path || "").replace(/\/+$/, "");
    var qIdx = path.indexOf("?");
    if (qIdx >= 0) path = path.substring(0, qIdx);
    var parts = path.split("/");
    return parts[parts.length - (index || 1)];
}

function genId() {
    return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, function (c) {
        var r = (Math.random() * 16) | 0;
        var v = c === "x" ? r : (r & 0x3) | 0x8;
        return v.toString(16);
    });
}

function query(sql, params) {
    var result = Host.dbQuery(sql, params ? JSON.stringify(params) : null);
    if (!result || result.indexOf("error:") === 0) return null;
    return JSON.parse(result);
}

function exec(sql, params) {
    var result = Host.dbExecute(sql, params ? JSON.stringify(params) : null);
    return JSON.parse(result);
}

// ── GET /products ────────────────────────────────────────────

Plugin.listProducts = function(input) {
    var rows = query("SELECT id, name, slug, price, compare_at_price, stock, sku, images, featured FROM products WHERE status = 'published' ORDER BY created_at DESC LIMIT 50");
    if (!rows) return ok(err(500, "query failed"));
    return ok({ items: rows, total: rows.length });
}

// ── GET /products/:id ────────────────────────────────────────

Plugin.getProduct = function(input) {
    var id = routeParam(input);
    var rows = query("SELECT id, name, slug, description, price, compare_at_price, stock, sku, images, featured, weight FROM products WHERE id = ? AND status = 'published'", [id]);
    if (!rows) return ok(err(500, "query failed"));
    if (rows.length === 0) return ok(err(404, "product not found"));
    return ok(rows[0]);
}

// ── GET /cart ────────────────────────────────────────────────

Plugin.viewCart = function(input) {
    var data = parseBody(input);
    var userId = data.user_id;
    if (!userId) return ok(err(400, "user_id required"));
    var rows = query("SELECT c.id as cart_id, c.product_id, c.quantity, p.name, p.price, p.stock, p.sku FROM cart_items c LEFT JOIN products p ON c.product_id = p.id WHERE c.user_id = ?", [userId]);
    if (!rows) return ok(err(500, "query failed"));
    var total = 0;
    for (var i = 0; i < rows.length; i++) {
        rows[i].subtotal = (rows[i].price || 0) * (rows[i].quantity || 0);
        total += rows[i].subtotal;
    }
    return ok({ items: rows, total: total });
}

// ── POST /cart ───────────────────────────────────────────────

Plugin.addToCart = function(input) {
    var data = parseBody(input);
    var userId = data.user_id;
    var productId = data.product_id;
    var quantity = data.quantity || 1;
    if (!userId || !productId) return ok(err(400, "user_id and product_id required"));

    var products = query("SELECT id, name, price, stock, status FROM products WHERE id = ?", [productId]);
    if (!products) return ok(err(500, "query failed"));
    if (products.length === 0) return ok(err(404, "product not found"));
    if (products[0].status !== "published") return ok(err(400, "product not available"));
    if (products[0].stock < quantity) return ok(err(400, "insufficient stock"));

    var existing = query("SELECT id, quantity FROM cart_items WHERE user_id = ? AND product_id = ?", [userId, productId]);
    if (!existing) return ok(err(500, "query failed"));

    if (existing.length > 0) {
        var newQty = existing[0].quantity + quantity;
        var r = exec("UPDATE cart_items SET quantity = ? WHERE id = ?", [newQty, existing[0].id]);
        if (r.error) return ok(err(500, r.error));
    } else {
        var id = genId();
        var now = new Date().toISOString();
        var r = exec("INSERT INTO cart_items (id, tenant_id, user_id, product_id, quantity, created_at, updated_at) VALUES (?, 'default', ?, ?, ?, ?, ?)", [id, userId, productId, quantity, now, now]);
        if (r.error) return ok(err(500, r.error));
    }
    return ok({ added: true });
}

// ── DELETE /cart ─────────────────────────────────────────────

Plugin.clearCart = function(input) {
    var data = parseBody(input);
    if (!data.user_id) return ok(err(400, "user_id required"));
    var r = exec("DELETE FROM cart_items WHERE user_id = ?", [data.user_id]);
    if (r.error) return ok(err(500, r.error));
    return ok({ cleared: true, rows_affected: r.rows_affected });
}

// ── PUT /cart/:id ────────────────────────────────────────────

Plugin.updateCartItem = function(input) {
    var itemId = routeParam(input);
    var data = parseBody(input);
    if (!itemId) return ok(err(400, "item id required"));
    if (!data.quantity || data.quantity < 1) return ok(err(400, "quantity must be >= 1"));
    var r = exec("UPDATE cart_items SET quantity = ? WHERE id = ?", [data.quantity, itemId]);
    if (r.error) return ok(err(500, r.error));
    if (r.rows_affected === 0) return ok(err(404, "cart item not found"));
    return ok({ updated: true });
}

// ── DELETE /cart/:id ─────────────────────────────────────────

Plugin.removeCartItem = function(input) {
    var itemId = routeParam(input);
    if (!itemId) return ok(err(400, "item id required"));
    var r = exec("DELETE FROM cart_items WHERE id = ?", [itemId]);
    if (r.error) return ok(err(500, r.error));
    if (r.rows_affected === 0) return ok(err(404, "cart item not found"));
    return ok({ removed: true });
}

// ── POST /checkout (事务) ────────────────────────────────────

Plugin.checkout = function(input) {
    var data = parseBody(input);
    var userId = data.user_id;
    if (!userId) return ok(err(400, "user_id required"));

    var cartItems = query("SELECT c.id as cart_id, c.product_id, c.quantity, p.name, p.price, p.stock, p.status FROM cart_items c LEFT JOIN products p ON c.product_id = p.id WHERE c.user_id = ?", [userId]);
    if (!cartItems) return ok(err(500, "query failed"));
    if (cartItems.length === 0) return ok(err(400, "cart is empty"));

    for (var i = 0; i < cartItems.length; i++) {
        var item = cartItems[i];
        if (!item.name) return ok(err(400, "product " + item.product_id + " not found"));
        if (item.status !== "published") return ok(err(400, "product " + item.name + " not available"));
        if (item.stock < item.quantity) return ok(err(400, "insufficient stock for " + item.name));
    }

    var totalAmount = 0;
    var orderItems = [];
    for (var i = 0; i < cartItems.length; i++) {
        var item = cartItems[i];
        var subtotal = item.price * item.quantity;
        totalAmount += subtotal;
        orderItems.push({ product_id: item.product_id, product_name: item.name, price: item.price, quantity: item.quantity, subtotal: subtotal });
    }

    var beginResult = JSON.parse(Host.dbBegin());
    if (!beginResult.ok) return ok(err(500, "failed to begin transaction"));

    var orderId = genId();
    var orderNo = "ORD-" + Date.now().toString(36).toUpperCase() + "-" + Math.random().toString(36).substring(2, 6).toUpperCase();
    var shippingJson = data.shipping_address ? JSON.stringify(data.shipping_address) : "";

    var r = exec(
        "INSERT INTO orders (id, tenant_id, order_no, user_id, status, total_amount, shipping_address, note, created_at, updated_at) VALUES (?, 'default', ?, ?, 'pending', ?, ?, ?, ?, ?)",
        [orderId, orderNo, userId, totalAmount, shippingJson, data.note || "", new Date().toISOString(), new Date().toISOString()]
    );
    if (r.error) {
        Host.dbRollback();
        return ok(err(500, "create order failed: " + r.error));
    }

    for (var i = 0; i < orderItems.length; i++) {
        var oi = orderItems[i];
        var oiId = genId();
        var r2 = exec(
            "INSERT INTO order_items (id, order_id, product_id, product_name, price, quantity, subtotal) VALUES (?, ?, ?, ?, ?, ?, ?)",
            [oiId, orderId, oi.product_id, oi.product_name, oi.price, oi.quantity, oi.subtotal]
        );
        if (r2.error) {
            Host.dbRollback();
            return ok(err(500, "create order item failed: " + r2.error));
        }

        var r3 = exec("UPDATE products SET stock = stock - ? WHERE id = ? AND stock >= ?", [oi.quantity, oi.product_id, oi.quantity]);
        if (r3.error || r3.rows_affected === 0) {
            Host.dbRollback();
            return ok(err(500, "stock deduction failed for " + oi.product_name));
        }
    }

    exec("DELETE FROM cart_items WHERE user_id = ?", [userId]);

    var commitResult = JSON.parse(Host.dbCommit());
    if (!commitResult.ok) return ok(err(500, "commit failed"));

    return ok({
        order_id: orderId,
        order_no: orderNo,
        status: "pending",
        total_amount: totalAmount,
        items_count: orderItems.length,
    });
}

// ── GET /orders ──────────────────────────────────────────────

Plugin.listOrders = function(input) {
    var data = parseBody(input);
    if (!data.user_id) return ok(err(400, "user_id required"));
    var rows = query("SELECT id, order_no, status, total_amount, note, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC LIMIT 50", [data.user_id]);
    if (!rows) return ok(err(500, "query failed"));
    return ok({ items: rows, total: rows.length });
}

// ── GET /orders/:id ──────────────────────────────────────────

Plugin.getOrder = function(input) {
    var orderId = routeParam(input);
    var data = parseBody(input);
    if (!orderId) return ok(err(400, "order id required"));
    var orders = query("SELECT id, order_no, user_id, status, total_amount, shipping_address, note, paid_at, shipped_at, created_at FROM orders WHERE id = ?", [orderId]);
    if (!orders) return ok(err(500, "query failed"));
    if (orders.length === 0) return ok(err(404, "order not found"));
    var order = orders[0];
    if (data.user_id && order.user_id !== data.user_id) return ok(err(403, "forbidden"));

    var items = query("SELECT id, product_id, product_name, price, quantity, subtotal FROM order_items WHERE order_id = ?", [orderId]);
    order.items = items || [];
    return ok(order);
}
