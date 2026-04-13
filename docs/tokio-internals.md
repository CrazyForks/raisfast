# Tokio 内部架构深度剖析

## 目录

1. [Runtime 架构](#1-runtime-架构)
2. [Scheduler 调度器](#2-scheduler-调度器)
3. [Task 系统](#3-task-系统)
4. [I/O 驱动 (mio/epoll)](#4-io-驱动)
5. [Timer 时间轮](#5-timer-时间轮)
6. [异步网络 I/O](#6-异步网络-io)
7. [sync 同步原语](#7-sync-同步原语)
8. [spawn / block_in_place](#8-spawn--block_in_place)

---

## 1. Runtime 架构

Tokio 的 Runtime 是整个异步运行时的入口点，它由三大核心组件组成：**Scheduler**（调度器）、**I/O Driver**（I/O 驱动）和 **Timer Driver**（定时器驱动）。此外还有一个 **Blocking Pool**（阻塞线程池）用于处理阻塞操作。

### 1.1 核心数据结构

```
Runtime
├── Handle (引用计数，可克隆，跨线程共享)
│   ├── sender → Spawner (向调度器投递任务)
│   ├── io_handle → IoDriver Handle
│   ├── time_handle → TimerDriver Handle
│   └── blocking_spawner → BlockingPool Handle
├── Scheduler
│   ├── MultiThread (多线程工作窃取调度器)
│   │   ├── workers: Vec<Worker>  (N 个 worker 线程)
│   │   ├── global: GlobalQueue   (全局任务队列)
│   │   └── remotes: Vec<Remote>  (外部投递点)
│   └── CurrentThread (单线程调度器)
│       ├── queue: Deque<task::Notified>  (本地任务队列)
│       └── LocalSet 支持
└── BlockingPool
    ├── workers: Vec<BlockingThread>
    └── queue: Mutex<Vec<task::UnownedTask>>
```

### 1.2 Multi-threaded Scheduler vs Current-thread Scheduler

**Multi-threaded Scheduler**：
- 启动时创建 `N` 个 worker 线程（默认等于 CPU 核心数）
- 每个 worker 线程拥有自己的 **local queue**（有界队列，容量 256）
- 共享一个 **global queue**（无界队列）
- 采用 **work-stealing**（工作窃取）算法实现负载均衡
- 适合生产环境、高并发 I/O 密集型场景

**Current-thread Scheduler**：
- 只有一个线程运行所有任务
- 没有 work-stealing，没有 local/global queue 的区分
- 任务按 FIFO 顺序依次执行
- 适合轻量级场景、嵌入式或测试环境

### 1.3 Runtime 启动流程

```
tokio::runtime::Builder::new_multi_thread()
       │
       ▼
┌──────────────────────────┐
│  Builder 配置阶段         │
│  - worker_threads(n)      │
│  - max_blocking_threads   │
│  - enable_all()           │
│  - thread_stack_size      │
└────────────┬─────────────┘
             │
             ▼
┌──────────────────────────┐
│  build() 构造阶段         │
│  1. 创建 I/O Driver       │
│  2. 创建 Timer Driver     │
│  3. 创建 Blocking Pool    │
│  4. 创建 Scheduler        │
│  5. 封装为 Runtime        │
└────────────┬─────────────┘
             │
             ▼
┌──────────────────────────┐
│  block_on() 启动阶段      │
│  1. 启动 worker 线程      │
│  2. 在当前线程轮转运行     │
│  3. 驱动 I/O + Timer      │
│  4. 阻塞直到 Future 完成  │
└──────────────────────────┘
```

### 1.4 Worker Thread Pool 生命周期

```
Worker Thread 生命周期:
┌───────────┐     ┌──────────┐     ┌──────────────┐     ┌──────────┐
│  等待任务  │────►│  运行任务 │────►│  等待 I/O    │────►│  被唤醒   │
│  (parking) │◄───│ (polling) │     │  (epoll_wait)│────►│  (unpark) │
└───────────┘     └──────────┘     └──────────────┘     └──────────┘
      ▲                                                        │
      │                                                        │
      └────────────────────────────────────────────────────────┘
                     (任务队列为空，再次等待)
```

每个 worker 线程的核心循环：

```
loop {
    // 1. 从 local queue 取任务
    if let Some(task) = local_queue.pop() {
        poll(task);                    // 运行任务
        continue;
    }
    // 2. 从 global queue 批量偷取
    if let Some(tasks) = global_queue.steal() {
        local_queue.push_batch(tasks);
        continue;
    }
    // 3. 从其他 worker 窃取
    if let Some(task) = steal_from_others() {
        poll(task);
        continue;
    }
    // 4. 尝试收割 I/O 和 Timer 事件
    park();  // 阻塞等待新事件
}
```

### 1.5 Blocking Pool

Blocking Pool 独立于异步调度器，是一组专用于阻塞操作的 OS 线程：

- 默认核心线程数 512（`max_blocking_threads`）
- 线程空闲 10 秒后自动回收
- 当核心线程全忙且未达上限时，创建新线程
- 用于 `spawn_blocking` 和 `block_in_place`

---

## 2. Scheduler 调度器

### 2.1 Work-Stealing 算法

Work-stealing 是 Tokio 多线程调度器的核心负载均衡策略。基本思想：**当一个 worker 的任务队列空了，就去"偷"其他 worker 的任务来执行**。

### 2.2 队列架构

```
┌──────────────────────────────────────────────────────────────┐
│                      Global Queue (无界)                      │
│   [TaskA] → [TaskB] → [TaskC] → [TaskD] → [TaskE] → ...     │
└──────┬───────────────────────────────────────────────────────┘
       │  steal (批量取 128 个)
       ▼
┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐
│ Worker 0 │  │ Worker 1 │  │ Worker 2 │  │ Worker 3 │
│ Local Q  │  │ Local Q  │  │ Local Q  │  │ Local Q  │
│(有界 256)│  │(有界 256)│  │(有界 256)│  │(有界 256)│
│          │  │          │  │          │  │          │
│ [T1][T2] │  │ [T5][T6] │  │  (空)    │  │ [T9][T10]│
│ [T3][T4] │  │ [T7][T8] │  │          │  │          │
└────┬─────┘  └────┬─────┘  └────┬─────┘  └────┬─────┘
     │              │              │              │
     │              │    steal half│              │
     │              │◄─────────────┘              │
     │              ▼                             │
     │         Worker 2 从                        │
     │         Worker 1 窃取了 [T7][T8]           │
```

### 2.3 任务分发优先级

当 `tokio::spawn(task)` 被调用时，任务分发遵循以下优先级：

```
tokio::spawn(task)
       │
       ▼
┌────────────────────────┐     是      ┌──────────────────┐
│ 当前线程是 Worker 吗？  │────────────►│ 推入 Local Queue │
└────────┬───────────────┘             └──────────────────┘
         │ 否
         ▼
┌────────────────────────┐    未满     ┌──────────────────┐
│ Local Queue 已满？     │────────────►│ 推入 Local Queue │
└────────┬───────────────┘             └──────────────────┘
         │ 已满
         ▼
┌────────────────────────┐
│ 推入 Global Queue      │
│ (通过 Remote sender)   │
└────────────────────────┘
```

### 2.4 Worker 搜索任务的优先级

```
Worker 搜索优先级 (从高到低):

┌─────────────────────────────────────────────┐
│ 优先级 1: Local Queue                       │
│   pop() → O(1), 缓存友好                    │
├─────────────────────────────────────────────┤
│ 优先级 2: Global Queue                      │
│   steal_batch() → 批量转移到 Local          │
├─────────────────────────────────────────────┤
│ 优先级 3: 其他 Worker 的 Local Queue        │
│   steal_half() → 窃取一半任务               │
├─────────────────────────────────────────────┤
│  优先级 4: I/O + Timer 驱动                 │
│   检查就绪事件，将对应任务加入 Local         │
├─────────────────────────────────────────────┤
│  优先级 5: Park (阻塞等待)                  │
│   所有队列空，调用 epoll_wait 等待          │
└─────────────────────────────────────────────┘
```

### 2.5 Work-Stealing 的窃取策略

窃取操作使用 **deque（双端队列）** 数据结构（来自 crossbeam-utils 的 `AtomicDequeue`）：

- **Owner（拥有者）**：从队列尾部（back）push/pop — LIFO 策略，利用缓存局部性
- **Stealer（窃取者）**：从队列头部（front）steal — FIFO 策略，窃取"最老"的任务

```
Local Queue (双端队列):
               Steal 方向 ◄──────────────
              ┌────┬────┬────┬────┬────┐
  (front)     │ T1 │ T2 │ T3 │ T4 │ T5 │  (back)
              └────┴────┴────┴────┴────┘
               ◄────────────── Owner 方向
               pop() / push() 从尾部操作
               steal() 从头部操作

窃取量 = max(1, 对方队列长度 / 2)
```

### 2.6 Global Queue 的作用

Global Queue 的存在是为了：
1. **外部线程投递**：非 worker 线程 `spawn` 的任务需要放入 Global Queue
2. **溢出处理**：当 Local Queue 满了，新任务溢出到 Global Queue
3. **公平性保证**：每隔 61 次本地调度，worker 会优先检查 Global Queue，防止饥饿

```
// tokio 内部的 61 倍数策略
const GLOBAL_QUEUE_INTERVAL: u16 = 61;

if tick % GLOBAL_QUEUE_INTERVAL == 0 {
    if let Some(task) = global_queue.pop() {
        return Some(task);  // 优先处理全局任务
    }
}
```

---

## 3. Task 系统

### 3.1 Task 的内存布局

Tokio 的 Task 实际上是一个 **自引用结构体**，在堆上分配。其核心类型是 `task::Harness`，它包装了用户的 Future。

```
┌─────────────────────────────────────────────────┐
│                   Task (堆分配)                   │
│                                                   │
│  ┌─────────────────────────────────────┐         │
│  │ Header (固定大小, 对齐到 cache line) │         │
│  │  - state: AtomicUsize              │         │
│  │    位标志:                           │         │
│  │    bit 0: NOTIFIED (已通知, 待 poll) │         │
│  │    bit 1: RUNNING  (正在 poll 中)   │         │
│  │    bit 2: COMPLETE (已完成)         │         │
│  │    bit 3: CANCELLED (被取消)        │         │
│  │    bit 4: JOIN_INTEREST (有人等结果) │         │
│  │    bit 5: JOIN_HANDLE_ALL_CLOSED    │         │
│  │  - vtable: *const Vtable            │         │
│  │  - owner: AtomicPtr<Shared>         │         │
│  └─────────────────────────────────────┘         │
│                                                   │
│  ┌─────────────────────────────────────┐         │
│  │ Future (用户定义的 async block)      │         │
│  │  包含所有 .await 点之间保存的状态     │         │
│  │  (即 Future 的状态机)               │         │
│  └─────────────────────────────────────┘         │
│                                                   │
│  ┌─────────────────────────────────────┐         │
│  │ Output (Future 完成后的结果)         │         │
│  │  Option<T> — 完成后写入             │         │
│  └─────────────────────────────────────┘         │
│                                                   │
│  ┌─────────────────────────────────────┐         │
│  │ Waker (用于唤醒此 task)              │         │
│  │  raw_waker 指向 Task 自身的 Header   │         │
│  │  (自引用！)                          │         │
│  └─────────────────────────────────────┘         │
└─────────────────────────────────────────────────┘
```

### 3.2 Task 状态机

```
                    spawn()
                       │
                       ▼
                ┌─────────────┐
                │   IDLE      │  state = 0 (无标志)
                │  (新创建)    │
                └──────┬──────┘
                       │ scheduler 将其加入队列
                       ▼
                ┌─────────────┐
         ┌─────│   NOTIFIED  │  state |= NOTIFIED
         │     │  (等待调度) │
         │     └──────┬──────┘
         │            │ worker 取出任务
         │            ▼
         │     ┌─────────────┐
         │     │   RUNNING   │  state |= RUNNING
         │  ┌─│  (正在 poll) │
         │  │  └──────┬──────┘
         │  │         │
         │  │         ├─── poll 返回 Ready ───► COMPLETE
         │  │         │                         state |= COMPLETE
         │  │         │
         │  │         └─── poll 返回 Pending ──► IDLE (清除 RUNNING)
         │  │                                    等待 Waker 唤醒
         │  │                                         │
         │  └─── Waker::wake() ───────────────────────┘
         │         │
         │         ▼
         │    重新进入 NOTIFIED
         │         │
         └─────────┘
```

### 3.3 Waker 机制

Waker 是 Rust 异步模型的核心。当 Future 返回 `Pending` 时，它必须安排在某个时刻通过 `Waker` 将自己重新唤醒。

```
Waker 的创建和流转:

┌──────────────┐     clone()     ┌──────────────┐
│  RawWaker    │───────────────►│  RawWaker    │
│  (指向 Task) │                │  (指向同一个  │
│              │                │   Task)      │
└──────┬───────┘                └──────┬───────┘
       │                               │
       │ waker.wake() / wake_by_ref()  │
       ▼                               ▼
┌──────────────────────────────────────────────┐
│  1. 将 Task 标记为 NOTIFIED                   │
│  2. 将 Task 推入调度队列 (Local 或 Global)    │
│  3. unpark worker 线程 (如果正在休眠)         │
└──────────────────────────────────────────────┘
```

`wake()` 和 `wake_by_ref()` 的区别：
- `wake()`：消耗 Waker，将 task 推入队列（可以跨线程）
- `wake_by_ref()`：不消耗 Waker，直接引用 task，效率更高

### 3.4 Future Poll 内部流程

当你写下 `let result = future.await` 时，编译器将这段代码转换为对 `Future::poll()` 的调用：

```
async fn my_task() {
    let stream = TcpStream::connect("127.0.0.1:8080").await;  // .await 点 1
    let data = stream.read(&mut buf).await;                    // .await 点 2
    stream.write_all(&data).await;                             // .await 点 3
}

编译器生成的状态机:
┌───────────┐     poll()      ┌───────────┐     poll()      ┌───────────┐
│  State 0  │───────────────►│  State 1  │───────────────►│  State 2  │
│ (connect) │    Ready(())   │  (read)   │   Ready(n)     │  (write)  │
└───────────┘                └───────────┘                └─────┬─────┘
     │ Pending                    │ Pending                    │ Ready
     ▼                            ▼                            ▼
  返回 Pending                返回 Pending                返回 Ready
  (注册 Waker                  (注册 Waker               (任务完成)
   到 I/O Driver)              到 I/O Driver)

每个 State 保存 .await 之间的所有局部变量
```

Tokio 的 `poll` 调用链：

```
Worker 线程执行:
  poll_task(task)
    │
    ├── task.state |= RUNNING
    │
    ├── context = Context::from_waker(&task.waker)
    │
    ├── match future.poll(context)     ◄─── 用户 Future
    │   │
    │   ├── Poll::Ready(output)
    │   │   ├── task.state |= COMPLETE
    │   │   ├── task.output = Some(output)
    │   │   └── 通知 JoinHandle
    │   │
    │   └── Poll::Pending
    │       ├── task.state &= !RUNNING
    │       └── (Waker 已在 Future 内部注册到某个驱动)
    │
    └── 处理下一个任务
```

---

## 4. I/O 驱动

### 4.1 Reactor 模式

Tokio 的 I/O 驱动基于经典的 **Reactor 模式**，底层使用 **mio** crate 抽象不同操作系统的多路复用 API：

```
┌──────────────────────────────────────────────────────┐
│                  操作系统层                            │
│   Linux: epoll  │  macOS: kqueue  │  Windows: IOCP   │
└───────────┬──────────────────────────────────────────┘
            │
            ▼
┌───────────────────────────┐
│      mio (底层抽象层)      │
│  - 统一 epoll/kqueue/IOCP│
│  - 提供 sys::Selector     │
│  - Token → Event 映射     │
└───────────┬───────────────┘
            │
            ▼
┌──────────────────────────────────────────────────┐
│           tokio::io::driver (I/O Driver)          │
│  - Reactor: 管理所有 I/O 资源的注册               │
│  - Registration: 每个 I/O 资源对应一个注册        │
│  - ReadyEvent: 就绪事件队列                       │
│  - 与 Scheduler 集成                              │
└──────────────────────────────────────────────────┘
```

### 4.2 I/O Driver 核心数据结构

```
IoDriver
├── reactor: Arc<Reactor>
│   ├── sys: Sys (底层 mio Selector)
│   │   └── epoll_fd / kqueue_fd
│   ├── dispatch: Slab<ScheduledIo>   // Token → ScheduledIo 映射
│   │   └── 每个 ScheduledIo 包含:
│   │       ├── readiness: AtomicUsize  // 当前就绪状态
│   │       ├── waiter: Waiter (Waker)
│   │       └── directions: [Direction; 2]
│   │           ├── read_direction
│   │           └── write_direction
│   └── token_counter: AtomicUsize     // 分配唯一 Token
│
└── 与 worker 线程共享 reactor 引用
```

### 4.3 I/O 资源注册流程

```
TcpStream::connect(addr) 被调用:
       │
       ▼
┌────────────────────────────────────────┐
│ 1. 调用 OS socket() 系统调用           │
│ 2. 设置 O_NONBLOCK (非阻塞模式)        │
│ 3. 调用 OS connect() (立即返回)        │
└───────────────┬────────────────────────┘
                │
                ▼
┌────────────────────────────────────────┐
│ Registration::new()                    │
│  1. 向 I/O Driver 注册此 socket        │
│  2. 分配唯一 Token (slab index)        │
│  3. epoll_ctl(ADD, fd, token)          │
│     或 kevent(fd, EV_ADD, token)       │
│  4. 创建 ScheduledIo 并存入 slab      │
└───────────────┬────────────────────────┘
                │
                ▼
┌────────────────────────────────────────┐
│ 返回 TcpStream (包含 Registration)    │
│ 此后, OS 就绪事件会触发此 socket      │
│ 对应的 ScheduledIo 被唤醒             │
└────────────────────────────────────────┘
```

### 4.4 事件循环：I/O Driver 与 Scheduler 的协作

```
Worker 线程主循环 (简化版):
┌───────────────────────────────────────────────────────┐
│                                                       │
│   ┌───────────────────────────────────────────┐       │
│   │  1. 从 Local/Global Queue 取任务并执行     │       │
│   │     (poll Future, 直到 Pending 或耗尽预算) │       │
│   └──────────────────┬────────────────────────┘       │
│                      │                                │
│                      ▼                                │
│   ┌───────────────────────────────────────────┐       │
│   │  2. 检查 I/O Driver 是否有就绪事件         │       │
│   │     process_io_events()                    │       │
│   │     → epoll_wait(timeout=0)  (非阻塞)     │       │
│   │     → 将就绪的 task 重新加入调度队列       │       │
│   └──────────────────┬────────────────────────┘       │
│                      │                                │
│                      ▼                                │
│   ┌───────────────────────────────────────────┐       │
│   │  3. 检查 Timer Driver 是否有过期定时器     │       │
│   │     process_timers()                       │       │
│   │     → 将超时的 task 重新加入调度队列       │       │
│   └──────────────────┬────────────────────────┘       │
│                      │                                │
│                      ▼                                │
│   ┌───────────────────────────────────────────┐       │
│   │  4. 如果没有任务了                         │       │
│   │     park() → epoll_wait(timeout>0) 阻塞   │       │
│   │     被唤醒后返回步骤 1                     │       │
│   └───────────────────────────────────────────┘       │
│                                                       │
└───────────────────────────────────────────────────────┘
```

### 4.5 Readiness 事件的流转

```
OS 层面:
  网络数据到达 → 网卡中断 → 内核标记 socket 可读
                           │
                           ▼
                 epoll_wait 返回 EPOLLIN 事件
                           │
                           ▼
mio 层面:
  Event { token: 42, readiness: READABLE }
                           │
                           ▼
tokio I/O Driver:
  查找 slab[42] → ScheduledIo
  设置 ScheduledIo.readiness |= READABLE
  调用 waiter.wake()
                           │
                           ▼
tokio Scheduler:
  将对应的 Task 加入调度队列
                           │
                           ▼
Worker 线程:
  取出 Task → poll(Future)
  Future 内部的 read() 现在会返回 Ready(data)
```

### 4.6 注册/注销 (Registration/Deregistration)

```
Registration 生命周期:

  TcpStream::new(socket)
        │
        ▼
  Registration::new()
        │
        ├── 分配 slab slot → token = slab.insert(scheduled_io)
        ├── epoll_ctl(EPOLL_CTL_ADD, fd, token, interests)
        └── 返回 Registration { token, slab_ref }
        │
        │  (使用中...多次 poll/wake 循环)
        │
        ▼
  Drop (TcpStream 被释放)
        │
        ├── epoll_ctl(EPOLL_CTL_DEL, fd)  // 从 epoll 移除
        ├── slab.remove(token)            // 释放 slab slot
        └── 完成
```

---

## 5. Timer 时间轮

### 5.1 为什么需要时间轮

普通的定时器实现（如 `BinaryHeap` 或 `BTreeMap`）插入/删除复杂度为 O(log n)。对于高并发场景（百万级超时），Tokio 采用了 **层级时间轮（Hierarchical Timing Wheel）**，实现 O(1) 的插入和接近 O(1) 的到期检测。

### 5.2 时间轮数据结构

Tokio 使用 **单层时间轮 + 溢出链表** 的简化设计（而非经典的多层级时间轮）：

```
TimerWheel 结构:
┌─────────────────────────────────────────────────┐
│               TimerWheel                         │
│                                                   │
│   slots: [Entry; WHEEL_SIZE]  (WHEEL_SIZE = 256) │
│                                                   │
│   ┌─────┬─────┬─────┬─────┬─────┬─────┬─────┐  │
│   │  0  │  1  │  2  │  3  │ ... │ 254 │ 255 │  │
│   └──┬──┴─────┴──┬──┴─────┴─────┴─────┴─────┘  │
│      │           │                               │
│      ▼           ▼                               │
│   Entry      Entry                               │
│   ┌────┐     ┌────┐                              │
│   │T1  │────►│T2  │ (链表)                       │
│   └────┘     └────┘                              │
│                                                   │
│   elapsed: Instant  (时间基准)                    │
│   当前指针位置: elapsed → slot_index              │
└─────────────────────────────────────────────────┘

每个 slot 包含一个双向链表 (使用 intrusive list, 无额外堆分配):
Entry {
    shared: Arc<TimerShared>,
    entries: LinkedListNode,
}
```

### 5.3 Timer 注册流程

```
tokio::time::sleep(duration) 被调用:
       │
       ▼
┌──────────────────────────────────────┐
│ Sleep::new(deadline)                 │
│  deadline = Instant::now() + duration│
└──────────────┬───────────────────────┘
               │
               ▼
┌──────────────────────────────────────┐
│ 注册到 TimerDriver:                  │
│  1. 计算目标 slot:                   │
│     slot = (deadline - elapsed)      │
│             % WHEEL_SIZE             │
│  2. 如果超时时间 > WHEEL_MAX:        │
│     放入溢出链表                     │
│  3. 否则:                            │
│     插入 slots[slot] 链表尾部        │
└──────────────┬───────────────────────┘
               │
               ▼
┌──────────────────────────────────────┐
│ Sleep::poll()                        │
│  if now >= deadline:                 │
│    return Poll::Ready(())            │
│  else:                               │
│    注册 Waker 到 TimerEntry          │
│    return Poll::Pending              │
└──────────────────────────────────────┘
```

### 5.4 Timer 到期检测

```
每次 TimerDriver 被轮转时:

current_time = Instant::now()
target_slot = (current_time - elapsed) % WHEEL_SIZE

遍历从上次位置到当前位置之间的所有 slot:
┌─────────────────────────────────────────┐
│ for slot in prev_slot..=target_slot:     │
│   for entry in slots[slot]:              │
│     if entry.deadline <= current_time:   │
│       entry.waker.wake()  // 唤醒任务   │
│       remove(entry)                      │
│     else:                                │
│       // 未到期，留在链表中或重新插入    │
│       reinsert(entry)                    │
└─────────────────────────────────────────┘

轮转过程可视化:
           prev_slot          target_slot
               ▼                  ▼
   [0][1][2][3][4][5]...[254][255]
               ◄──── 旋转指针 ────►
               遍历中间所有 slot，触发已过期的定时器
```

### 5.5 层级时间轮概念 (扩展理解)

虽然 Tokio 使用的是简化的单层时间轮，但理解经典层级时间轮有助于深入理解：

```
经典 4 层时间轮 (类似钟表):
层 0: 毫秒级  (256 slots, 每格 1ms,  总范围 256ms)
层 1: 秒级    (64 slots,  每格 256ms, 总范围 ~16s)
层 2: 分钟级  (64 slots,  每格 ~16s,  总范围 ~17min)
层 3: 小时级  (64 slots,  每格 ~17min, 总范围 ~18h)

插入:
  timeout = 3ms    → 层 0, slot 3
  timeout = 500ms  → 层 1, slot 1 (关联到层 0 的第 0 slot)
  timeout = 30s    → 层 2, slot 1

当层 0 转满一圈 → 层 1 前进一格 → 层 1 对应 slot 的任务降级到层 0
```

---

## 6. 异步网络 I/O

### 6.1 TcpStream 内部实现

```
TcpStream 架构:
┌──────────────────────────────────────────┐
│              TcpStream                    │
│                                           │
│  ┌──────────────────────────┐            │
│  │  inner: Arc<TcpStreamInner>│           │
│  │  ┌────────────────────┐  │            │
│  │  │  mio::net::TcpStream │ │            │
│  │  │  (非阻塞 socket fd)  │ │            │
│  │  ├────────────────────┤  │            │
│  │  │  Registration       │ │            │
│  │  │  (I/O Driver 注册)  │ │            │
│  │  └────────────────────┘  │            │
│  └──────────────────────────┘            │
└──────────────────────────────────────────┘
```

### 6.2 TcpStream::connect 流程

```
TcpStream::connect(addr)
       │
       ▼
┌──────────────────────────────────────────────┐
│ 1. socket() → 创建非阻塞 socket              │
│    setsockopt(O_NONBLOCK)                    │
└──────────────┬───────────────────────────────┘
               │
               ▼
┌──────────────────────────────────────────────┐
│ 2. connect(addr)                             │
│    → 返回 EINPROGRESS (非阻塞连接进行中)     │
└──────────────┬───────────────────────────────┘
               │
               ▼
┌──────────────────────────────────────────────┐
│ 3. 注册到 I/O Driver (EPOLLOUT 事件)         │
│    等待 socket 变为可写(连接建立)             │
└──────────────┬───────────────────────────────┘
               │
               ▼
┌──────────────────────────────────────────────┐
│ 4. poll(): 返回 Pending                      │
│    Waker 注册到 write_direction              │
└──────────────┬───────────────────────────────┘
               │
               ▼ (一段时间后)
┌──────────────────────────────────────────────┐
│ 5. OS: TCP 三次握手完成                       │
│    epoll 返回 EPOLLOUT                       │
│    → I/O Driver 唤醒 Waker                   │
│    → Task 重新被调度                         │
└──────────────┬───────────────────────────────┘
               │
               ▼
┌──────────────────────────────────────────────┐
│ 6. 再次 poll(): 检查 SO_ERROR               │
│    如果无错误 → 连接成功                     │
│    返回 Poll::Ready(Ok(TcpStream))           │
└──────────────────────────────────────────────┘
```

### 6.3 TcpStream::read 流程

```
stream.read(&mut buf)
       │
       ▼
┌──────────────────────────────────────┐
│ readiness = registration.readiness() │
│ (检查当前就绪状态, AtomicUsize)       │
└──────────────┬───────────────────────┘
               │
       ┌───────┴────────┐
       │                │
   READABLE           未就绪
   (可读)             (未就绪)
       │                │
       ▼                ▼
┌──────────────┐  ┌──────────────────────┐
│ 直接调用     │  │ 注册 Waker 到         │
│ OS read()    │  │ read_direction        │
│ 返回数据     │  │ 等待 EPOLLIN          │
│ Ready(n)     │  │ 返回 Pending          │
└──────────────┘  └──────────────────────┘
                          │
                          ▼ (数据到达)
                   ┌──────────────────────┐
                   │ epoll 返回 EPOLLIN   │
                   │ I/O Driver 唤醒 task │
                   │ 再次 poll → 可读     │
                   └──────────────────────┘
```

### 6.4 TcpListener::accept 流程

```
TcpListener::bind(addr)
       │
       ▼
┌──────────────────────────────────────┐
│ 1. socket() + bind() + listen()      │
│ 2. 设置 O_NONBLOCK                   │
│ 3. 注册到 I/O Driver (EPOLLIN)       │
│    监听可读事件 = 有新连接            │
└──────────────────────────────────────┘

listener.accept().await:
       │
       ▼
┌──────────────────────────────────────┐
│ 检查 readiness 是否包含 READABLE     │
│                                      │
│ 是 → OS accept() → 非阻塞接受连接   │
│     → 返回 Ready((TcpStream, addr))  │
│                                      │
│ 否 → 注册 Waker, 返回 Pending       │
│     → EPOLLIN 到达时被唤醒          │
└──────────────────────────────────────┘

accept 循环优化 (accept4 / fast path):
  在 Linux 上使用 accept4() 系统调用
  一次 accept4 = accept + fcntl(O_NONBLOCK)
  减少 50% 系统调用次数
```

### 6.5 拆分读写 (TcpStream split)

```
TcpStream (可同时读写)
       │
       ├── split() ──► (ReadHalf, WriteHalf)
       │                   │          │
       │                   ▼          ▼
       │               只能 read   只能 write
       │               各自注册    各自注册
       │               EPOLLIN    EPOLLOUT
       │               Waker      Waker
       │
       │  内部共享同一个 Arc<TcpStreamInner>
       │  读写方向独立追踪就绪状态
       │
       └── into_split() ──► (OwnedReadHalf, OwnedWriteHalf)
               拥有所有权，生命周期与 TcpStream 相同
```

---

## 7. sync 同步原语

### 7.1 tokio::sync::Mutex

Tokio 的 Mutex 与 `std::sync::Mutex` 的关键区别：**Tokio Mutex 的 lock() 是 async 函数，在等待锁时不会阻塞线程**。

```
Mutex<T> 内部:
┌─────────────────────────────────────┐
│  Mutex<T>                            │
│  ┌──────────────────────────────┐   │
│  │  s: Semaphore (控制并发度)   │   │
│  │  permits: 初始为 1           │   │
│  └──────────────────────────────┘   │
│  ┌──────────────────────────────┐   │
│  │  c: UnsafeCell<T>            │   │
│  │  (内部可变性, 运行时检查借用)│   │
│  └──────────────────────────────┘   │
└─────────────────────────────────────┘

lock().await 流程:
┌──────────────────┐
│ mutex.lock()     │
└────────┬─────────┘
         ▼
┌──────────────────────────┐     获取成功     ┌──────────────┐
│ semaphore.acquire(1)     │───────────────►│ MutexGuard   │
│ (尝试获取 1 个 permit)  │                 │ (持有锁)     │
└──────────┬───────────────┘                 └──────┬───────┘
           │ 获取失败                                │ drop()
           ▼                                        ▼
┌──────────────────────────┐             ┌──────────────────┐
│ 注册 Waker 到 Semaphore  │             │ semaphore.add(1) │
│ 等待队列                 │             │ 唤醒下一个等待者 │
│ 返回 Pending             │             └──────────────────┘
└──────────────────────────┘
```

### 7.2 tokio::sync::Semaphore

Semaphore 是 Tokio 同步原语的基础组件，Mutex 内部就使用了 Semaphore。

```
Semaphore 内部数据结构:
┌───────────────────────────────────────────┐
│  Semaphore                                 │
│  ┌───────────────────────────────────┐    │
│  │  ll: DoublyLinkedList<Waiter>     │    │
│  │  (等待队列, intrusive linked list)│    │
│  │                                   │    │
│  │  [Waiter1]◄──►[Waiter2]◄──►[W3]  │    │
│  │   permits=3     permits=5     p=2 │    │
│  └───────────────────────────────────┘    │
│  permits: AtomicUsize  (当前可用 permit)  │
└───────────────────────────────────────────┘

acquire(n) 流程:
       │
       ▼
┌───────────────────────────────────┐
│ loop {                             │
│   curr = permits.load()            │
│   if curr >= n:                    │
│     if permits.compare_exchange(   │
│       curr, curr - n).is_ok():     │
│       return Acquired  // 快速路径 │
│   else:                            │
│     加入等待队列                    │
│     return Pending                 │
│ }                                  │
└───────────────────────────────────┘

add(n) (释放 permits):
       │
       ▼
┌───────────────────────────────────┐
│ 1. permits += n                   │
│ 2. 遍历等待队列:                  │
│    if waiter.permits <= permits:  │
│      permits -= waiter.permits    │
│      waiter.wake()                │
│      移除此 waiter                │
└───────────────────────────────────┘
```

### 7.3 Channel — mpsc

`mpsc` 是多生产者、单消费者通道。Tokio 提供了 **有界（bounded）** 和 **无界（unbounded）** 两种实现。

```
mpsc::channel(capacity) 内部结构:
┌─────────────────────────────────────────────────────┐
│                   Channel (Shared)                    │
│  ┌─────────────────────────────────────────────┐    │
│  │  buf: VecDeque<T>   (环形缓冲区)            │    │
│  │  ┌───┬───┬───┬───┬───┬───┬───┬───┐         │    │
│  │  │   │ T1│ T2│ T3│   │   │   │   │         │    │
│  │  └───┴───┴───┴───┴───┴───┴───┴───┘         │    │
│  │        ▲           ▲                         │    │
│  │       head        tail                       │    │
│  ├─────────────────────────────────────────────┤    │
│  │  tx_count: AtomicUsize  (发送者计数)         │    │
│  │  closed: bool                               │    │
│  ├─────────────────────────────────────────────┤    │
│  │  waiters: LinkedList<Waiter>                │    │
│  │  (接收者等待队列 + 发送者等待队列)           │    │
│  │  tx_waiters: 等待 buf 有空间                │    │
│  │  rx_waiters: 等待 buf 有数据                │    │
│  ├─────────────────────────────────────────────┤    │
│  │ Semaphore (bounded 时用于流量控制)          │    │
│  │  初始 permits = capacity                    │    │
│  └─────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────┘
```

**send 流程 (bounded):**

```
tx.send(value).await
       │
       ▼
┌──────────────────────────────────────┐
│ 1. semaphore.acquire(1)              │
│    (获取 1 个 permit = 占用 1 个槽位)│
│    如果 buf 满 → Pending 等待       │
└──────────────┬───────────────────────┘
               │ 成功
               ▼
┌──────────────────────────────────────┐
│ 2. lock(buf)                         │
│    buf.push_back(value)              │
│    唤醒 rx_waiter (如果有)           │
└──────────────────────────────────────┘
```

**recv 流程:**

```
rx.recv().await
       │
       ▼
┌──────────────────────────────────────┐
│ 1. lock(buf)                         │
│    if buf.pop_front().is_some():     │
│      semaphore.add(1) // 释放 permit │
│      return Ready(Some(value))       │
│    elif closed:                      │
│      return Ready(None)              │
│    else:                             │
│      加入 rx_waiters                 │
│      return Pending                  │
└──────────────────────────────────────┘
```

### 7.4 Channel — oneshot

`oneshot` 是单发送、单接收通道，只传一个值。极其轻量。

```
oneshot::channel() 内部:
┌─────────────────────────────────────┐
│  Inner<T>  (Arc<AtomicInner<T>>)     │
│  ┌─────────────────────────────┐    │
│  │  value: UnsafeCell<MaybeUninit<T>>│
│  │  state: AtomicU8              │    │
│  │    EMPTY = 0                  │    │
│  │    VALUE = 1  (已发送值)      │    │
│  │    CLOSED = 2 (发送者丢弃)    │    │
│  │    CONSUMED = 3 (已消费)      │    │
│  │  rx_task: AtomicWaker         │    │
│  │  tx_task: AtomicWaker         │    │
│  └─────────────────────────────┘    │
└─────────────────────────────────────┘

发送流程:
  tx.send(value)
    │
    ├── state.compare_exchange(EMPTY, VALUE)
    │   ├── 成功 → inner.value = value → rx_task.wake()
    │   └── 失败 → (接收者已关闭) 返回 Err(value)
```

### 7.5 Channel — broadcast

`broadcast` 支持多生产者、多消费者，每个消费者都能收到所有消息。

```
broadcast::channel(capacity) 内部:
┌─────────────────────────────────────────────────┐
│  Channel<T>                                      │
│  ┌─────────────────────────────────────────┐    │
│  │  buf: Slab<(T, RefCount)>  (环形缓冲)   │    │
│  │  每个 slot 存储消息 + 引用计数           │    │
│  │                                          │    │
│  │  tail: u64  (写入位置)                   │    │
│  │  head: u64  (最老消息位置)               │    │
│  ├─────────────────────────────────────────┤    │
│  │  receivers: Slab<Receiver>               │    │
│  │  每个 Receiver 记录自己的 next_seen: u64 │    │
│  │  (表示它已读到哪里)                       │    │
│  └─────────────────────────────────────────┘    │
└─────────────────────────────────────────────────┘

发送流程:
  tx.send(value)
    │
    ├── value 写入 buf[tail]
    │   ref_count = receivers.len()  // 每个 receiver 一份引用
    │   tail += 1
    │
    ├── 遍历所有 receivers:
    │   receiver.next_seen < tail → wake(receiver)
    │
    └── 如果 buf 溢出:
        丢弃最老的消息 (head++)
        慢 receiver 会收到 RecvError::Lagged(n)

每个 Receiver 独立消费:
  rx.recv()
    │
    ├── next_seen < tail → 读取 buf[next_seen], next_seen++
    │   ref_count -= 1
    │   if ref_count == 0 → 释放 slot
    │
    └── next_seen == tail → Pending (等待新消息)
```

### 7.6 同步原语与 Scheduler 的交互

所有 Tokio 同步原语的核心模式一致：

```
┌──────────────────────────────────────────────┐
│           通用 Waker 通知模式                 │
│                                               │
│  等待方:                                      │
│    1. 检查条件是否满足 (原子操作)             │
│    2. 不满足 → 将 Waker 存入等待队列          │
│    3. 再次检查 (防止 TOCTOU 竞态)             │
│    4. 仍然不满足 → 返回 Pending               │
│                                               │
│  通知方:                                      │
│    1. 更新共享状态                             │
│    2. 从等待队列取出 Waker                     │
│    3. waker.wake() → 任务重新加入调度队列     │
│    4. Scheduler 在某次轮转中重新 poll 该任务  │
└──────────────────────────────────────────────┘
```

---

## 8. spawn / block_in_place

### 8.1 tokio::spawn 工作原理

`tokio::spawn` 是将 Future 提交到 Tokio 调度器的核心 API。

```
tokio::spawn(async { ... })
       │
       ▼
┌──────────────────────────────────────────┐
│ 1. 获取当前 Runtime Handle               │
│    HANDLE.try_with(|h| h.clone())        │
│    (通过 thread-local 获取)              │
└──────────────┬───────────────────────────┘
               │
               ▼
┌──────────────────────────────────────────┐
│ 2. 将 Future 包装为 Task                 │
│    task = RawTask::new::<F>(future)      │
│    在堆上分配: Header + Future + Output  │
│    设置 vtable (poll/drop/read_output)   │
└──────────────┬───────────────────────────┘
               │
               ▼
┌──────────────────────────────────────────┐
│ 3. 标记为 NOTIFIED                       │
│    task.state |= NOTIFIED                │
└──────────────┬───────────────────────────┘
               │
               ▼
┌──────────────────────────────────────────┐
│ 4. 投递到调度器                          │
│    if 当前在 worker 线程:                │
│      → local_queue.push(task)            │
│      → 如果 local queue 满:             │
│        overflow 到 global queue          │
│    else:                                 │
│      → global_queue.push(task)           │
│      → unpark 一个 worker 线程           │
└──────────────┬───────────────────────────┘
               │
               ▼
┌──────────────────────────────────────────┐
│ 5. 返回 JoinHandle<F>                   │
│    (可以 .await 获取结果)                │
│    JoinHandle 本身也是一个 Future        │
└──────────────────────────────────────────┘
```

### 8.2 spawn 与 JoinHandle 的关系

```
                    Task
                 ┌────────┐
                 │ Header │
                 │ state  │◄───── JoinHandle 持有引用
                 │ vtable │
                 ├────────┤
                 │Future  │
                 ├────────┤
                 │Output  │──► 任务完成后, Output 写入此处
                 └────────┘
                    ▲
                    │
        JoinHandle.await:
        ┌───────────────────────────┐
        │ if task.state & COMPLETE: │
        │   读取 Output             │
        │   return Ready(output)    │
        │ else:                     │
        │   注册 Waker 到 Task      │
        │   return Pending          │
        │                           │
        │ (Task 完成时被唤醒)       │
        └───────────────────────────┘
```

### 8.3 spawn_blocking

`spawn_blocking` 用于在**专用阻塞线程池**中运行阻塞代码（如同步文件 I/O、CPU 密集计算、调用 C 库等）。

```
tokio::task::spawn_blocking(|| {
    // 阻塞代码, 例如:
    std::fs::read_to_string("file.txt")
})
       │
       ▼
┌──────────────────────────────────────────┐
│ 1. 将闭包包装为 BlockingTask             │
│    BlockingTask implements Future        │
│    poll() → 返回 Pending                 │
│    (闭包在独立线程中执行)                │
└──────────────┬───────────────────────────┘
               │
               ▼
┌──────────────────────────────────────────┐
│ 2. 提交到 BlockingPool                  │
│    pool.spawn(blocking_task)             │
└──────────────┬───────────────────────────┘
               │
               ▼
┌──────────────────────────────────────────┐
│ BlockingPool 分配策略:                   │
│                                          │
│ ┌──────────────────────────────┐        │
│ │ 检查是否有空闲的核心线程      │        │
│ │ (核心线程不会被回收)          │        │
│ └──────────┬───────────────────┘        │
│            │                             │
│     ┌──────┴──────┐                     │
│     │有            │无                    │
│     ▼              ▼                     │
│  唤醒空闲      创建新线程                │
│  核心线程      (如果未达 max 限制)       │
│                  │                       │
│                  │ 达到上限              │
│                  ▼                       │
│             放入等待队列                 │
│             (直到有线程空闲)             │
└──────────────────────────────────────────┘
               │
               ▼
┌──────────────────────────────────────────┐
│ 3. 阻塞线程执行闭包                      │
│    闭包执行完毕后:                        │
│    - 结果写入 BlockingTask               │
│    - 唤醒等待的 JoinHandle               │
│    - 线程回归线程池等待下一个任务          │
└──────────────────────────────────────────┘
```

### 8.4 block_in_place

`block_in_place` 是 Tokio 独有的 API，它允许在 **worker 线程上** 执行阻塞操作，而不需要切换到阻塞线程池。它通过"让出"当前 worker 线程的调度能力来避免阻塞整个调度器。

```
tokio::task::block_in_place(|| {
    // 阻塞代码
    std::thread::sleep(Duration::from_secs(5))
})
       │
       ▼
┌──────────────────────────────────────────────┐
│ 仅在 multi_thread scheduler 中可用           │
│                                              │
│ 1. 标记当前 worker 线程为 "blocking"         │
│    worker.transition_to_blocking()           │
│                                              │
│ 2. Worker "脱产":                            │
│    - 不再从队列取新任务                       │
│    - 将 local_queue 剩余任务转移到 global    │
│    - 通知调度器启动新的 worker 线程替代       │
│                                              │
│ 3. 在当前线程执行阻塞闭包                    │
│    (此时 worker 线程被阻塞, 但调度器          │
│     已有新 worker 替代它)                    │
│                                              │
│ 4. 阻塞闭包执行完毕                          │
│    worker.transition_from_blocking()         │
│    - 恢复正常调度                            │
│    - 如果已有足够 worker, 此线程可能退出     │
└──────────────────────────────────────────────┘
```

### 8.5 spawn_blocking vs block_in_place 对比

```
┌──────────────────┬────────────────────────┬────────────────────────┐
│                  │  spawn_blocking        │  block_in_place        │
├──────────────────┼────────────────────────┼────────────────────────┤
│ 执行线程         │  独立的阻塞线程         │  当前 worker 线程      │
│ 线程切换         │  有 (切换到阻塞线程)    │  无 (原地执行)         │
│ 需要运行时       │  有 Handle 即可         │  必须在 worker 线程上  │
│ 开销             │  较高 (线程切换)        │  较低 (无切换)         │
│ 影响            │  不影响调度器            │  暂时减少一个 worker  │
│ 适用场景         │  通用阻塞操作            │  短暂阻塞、热路径     │
│ Future 兼容     │  返回 'static Future    │  可以在 async fn 中用  │
│ 闭包要求         │  'static + Send        │  'static + Send       │
└──────────────────┴────────────────────────┴────────────────────────┘
```

### 8.6 完整的 spawn 调用链总结

```
用户代码                      tokio 内部                      OS 层面
═══════                      ══════════                      ══════

tokio::spawn(fut)────► 创建 Task (堆分配)
                       │
                       ├─► LocalQueue.push(task)
                       │    (或 GlobalQueue)
                       │
                       ▼
                 Worker 取出 task
                       │
                       ├─► task.poll()
                       │    │
                       │    ├─► user_future.poll(cx)
                       │    │         │
                       │    │    ┌────┴──────────────────┐
                       │    │    │                       │
                       │    │ Pending               Ready(output)
                       │    │    │                       │
                       │    │    ▼                       ▼
                       │    │ 注册 Waker             任务完成
                       │    │ 到某个驱动:            通知 JoinHandle
                       │    │                        释放 Task
                       │    │ ┌──────────────┐
                       │    │ │ I/O Driver   │──► epoll_ctl()
                       │    │ │ Timer Driver │──► timerfd / wheel
                       │    │ │ Channel      │──► waiter list
                       │    │ │ Semaphore    │──► waiter list
                       │    │ └──────────────┘
                       │    │
                       │    │ (事件到达时)
                       │    │ Waker.wake() ──► task 重新入队
                       │    │                    │
                       │    └────────────────────┘
                       │
                       ▼
                 Worker 继续下一个任务
                       │
                       ▼ (队列为空)
                 park() ──────────────────────► epoll_wait()
                                                 (阻塞等待事件)
                                                    │
                                                    │ (事件到达)
                                                    ▼
                                                 唤醒 Worker
                                                    │
                                                    ◄───────
```

---

## 总结

Tokio 的架构可以用一张总览图来概括：

```
┌─────────────────────────────────────────────────────────────────────┐
│                          tokio Runtime                               │
│                                                                      │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │                     Scheduler (调度器)                       │    │
│  │                                                              │    │
│  │  ┌────────┐  ┌────────┐  ┌────────┐        ┌───────────┐  │    │
│  │  │Worker 0│  │Worker 1│  │Worker 2│  ...   │Worker N-1 │  │    │
│  │  │Local Q │  │Local Q │  │Local Q │        │Local Q    │  │    │
│  │  └───┬────┘  └───┬────┘  └───┬────┘        └─────┬─────┘  │    │
│  │      │            │            │                   │         │    │
│  │      └────────────┴─────┬──────┴──────────────────┘         │    │
│  │                         │ Work Stealing                      │    │
│  │                    ┌────┴────┐                                │    │
│  │                    │Global Q │                                │    │
│  │                    └─────────┘                                │    │
│  └──────────────────────────────────────────────────────────────┘    │
│                                                                      │
│  ┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐  │
│  │   I/O Driver     │  │  Timer Driver    │  │  Blocking Pool   │  │
│  │   (epoll/kqueue) │  │  (Timing Wheel)  │  │  (OS Threads)    │  │
│  │                  │  │                  │  │                  │  │
│  │  Reactor 模式    │  │  层级时间轮      │  │  spawn_blocking  │  │
│  │  mio 抽象层      │  │  O(1) 注册/触发 │  │  block_in_place  │  │
│  └──────────────────┘  └──────────────────┘  └──────────────────┘  │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  Sync Primitives: Mutex | Semaphore | msc | oneshot | broadcast│   │
│  │  (所有原语通过 Waker 与 Scheduler 交互)                      │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                                                                      │
│  Handle (Cloneable, Send, Sync — 跨线程共享运行时引用)             │
└─────────────────────────────────────────────────────────────────────┘
```

**核心设计理念总结：**

1. **一切皆 Task**：所有异步工作最终都被包装为 Task，由 Scheduler 统一管理
2. **Work-Stealing 实现负载均衡**：多线程调度器通过窃取机制实现自动负载均衡，无需手动分配
3. **Reactor 模式驱动 I/O**：通过 epoll/kqueue/IOCP 统一抽象，实现高效的 I/O 多路复用
4. **Waker 是纽带**：Waker 将 I/O 驱动、Timer 驱动、同步原语与 Scheduler 连接起来
5. **无侵入式设计**：用户只需写 `async/await`，Tokio 在底层处理所有调度和 I/O 细节
6. **分层架构**：Runtime → Scheduler → Driver（I/O/Timer）→ OS，层次清晰，职责分明