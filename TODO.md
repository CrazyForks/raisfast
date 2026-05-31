# raisfast 电商模块 TODO

## P0 — 必做（阻塞上线）

### 1. ~~库存扣减/回补~~ ✅ 已完成
- 下单时扣减 `products.stock`（CAS `WHERE stock >= qty`，防超卖）
- 取消订单（用户/管理员）自动回补库存
- `tx_deduct_stock` / `tx_replenish_stock` 在事务内执行
- 28 个 order 测试 + 6 个 stock 测试全部通过

### 2. ~~订单超时自动关闭~~ ✅ 已完成
- Worker handler `ExpireOrders`，每 5 分钟扫描 `pending` 超时订单
- CAS 更新状态为 `expired`，自动回补库存
- 超时时间通过 `ORDER_EXPIRE_MINUTES` 配置（默认 30 分钟）
- 涉及：`src/worker/handlers/order_expire.rs`、`src/worker.rs`、`src/config/app.rs`

### 3. ~~地址关联到订单~~ ✅ 已完成
- `CreateOrderRequest` 新增 `shipping_address_id` / `billing_address_id`
- 下单时从 `user_addresses` 读取完整地址，自动填充 `shipping_address`、`buyer_name`、`buyer_phone`
- 地址所有权校验（必须属于下单用户）
- 涉及：`dto/order.rs`、`services/order.rs`

### 4. 商品评价系统
- 新表 `product_comments`（rating / title / content / images / admin_reply）
- 只有已完成订单（`status = 'completed'`）的用户才能评价
- 同一用户对同一商品同一订单只能评价一次（UNIQUE 约束）
- 管理员回复
- 评分聚合统计（平均分、分布）
- **状态：实现中**

### 5. 优惠券系统
- 新表 `coupons`（code / type: fixed|percent / value / min_order / max_uses / expires_at）
- 新表 `order_coupons`（关联订单与优惠券）
- 下单时验证并计算 `discount_amount`
- 涉及：新 `models/coupon.rs`、`services/coupon.rs`、修改 `services/order.rs`

### 6. 运费计算
- 新表 `shipping_templates`（按重量/件数/地区计费规则）
- `Product.shipping_template_id` 关联模板
- 下单时根据商品重量 + 收货地区计算 `shipping_amount`
- 涉及：新 `models/shipping_template.rs`、修改 `services/order.rs`

---

## P1 — 重要但非阻塞

- 商品搜索/筛选（按价格区间、属性、分类）
- 收藏/心愿单
- 多币种价格/汇率换算
- 售后退换流程（退货地址 + 物流跟踪）
- 订单导出（CSV/Excel）
