# SSH 工具跟不上开发节奏了：RSSH 的动态发现

传统 SSH 客户端有一个默认假设：你要连接的是一批稳定存在的机器。

所以它们的核心模型通常长这样：

- 新建一个 Profile
- 填 host、port、username、key
- 保存
- 下次从列表里点开

这套模型在服务器数量少、主机长期存在的年代没问题。但现在的开发现场已经变了。

你面对的不再只是几台固定服务器，而是一堆不断变化的运行目标：

- Docker 里今天启动了 12 个容器，明天名字和 ID 全变了
- Kubernetes 里 Pod 被滚动发布、重建、调度到别的节点
- 同一个开发者本机可能连着多个 Docker context、多个 kube context
- 后续还会有 AWS EC2、阿里云 ECS、Cloudflare Tunnel 这类外部资源

这时候还让用户手动建 Profile，本质上是在拿 1990 年代的主机清单模型，硬套今天的动态基础设施。

**不是用户懒，是工具落后。**

## 静态 Profile 不是错，错在把一切都当成静态 Profile

SSH Profile 解决的是固定入口：

```text
name -> host -> credential -> connect
```

它适合堡垒机、长期存在的开发机、数据库跳板、内网服务器。

但容器和 Pod 不是这种东西。它们的生命周期更短，数量更多，名字也经常带 hash。把每一个容器都做成一个 Profile，只会制造垃圾配置。

更糟的是，动态目标会污染静态数据：

- 一个已经退出的容器还留在列表里
- 一个被重建的 Pod 变成了死配置
- 收藏、排序、分组开始混入一堆临时对象
- 真正重要的长期连接反而被淹没

所以动态发现的第一条原则是：

> **发现结果不是配置。**

RSSH 只持久化“发现来源”，不持久化“发现结果”。

Docker container、Kubernetes pod 是实时结果。它们出现在 Home 里，可以搜索、排序、打开，但不进入 Profile 数据库，也不能收藏。

这是正确的数据结构。

## 动态发现保存的是来源，不是目标

动态发现的配置对象很小：

```text
source = platform + context + namespace + shell + enabled
```

Docker 来源关心：

- Docker CLI 是否存在
- Docker context
- 进入容器后使用什么 shell

Kubernetes 来源关心：

- kubectl CLI 是否存在
- kube context
- namespace，留空就是全部 namespace
- 进入 Pod 后使用什么 shell

RSSH 不会先 SSH 到宿主机，再在宿主机上跑 `docker ps`。那是错误抽象。

Docker 和 Kubernetes 已经有自己的 context 机制：

```bash
docker context ls
kubectl config get-contexts
```

开发者本来就是通过本机 Docker CLI / kubectl 访问远端环境。RSSH 只复用这个事实，不再发明一套新的远端探测协议。

这有几个好处：

- 不需要在服务器上安装 agent
- 不需要维护宿主机 Profile 和容器发现之间的隐藏依赖
- 不需要猜 Docker daemon 在哪台机器上
- 不绕过用户已经配置好的 Docker/kube 认证链路

工具应该复用现有工作方式，而不是逼用户为工具重建一套世界。

## Home 里应该打平展示，而不是再造一个入口

发现出来的目标不是二等公民。

一个 Docker 容器、一个 Kubernetes Pod，和一个 SSH Profile 一样，最终都是“可打开的终端目标”。

所以 Home 不应该变成：

```text
Profiles
Forwards
Dynamic Discovery
  Docker
  Kubernetes
    ...
```

这种层级会把用户重新拖回配置管理页面。

Home 应该展示的是用户下一秒要打开的东西：

```text
profile
forward
docker_exec
kubectl_exec
```

它们是平级目标。搜索、排序、打开逻辑也应该走同一套模型。

差异只在生命周期：

- Profile / Forward 是静态配置
- Docker / Kubernetes 目标是动态结果

这就是为什么 RSSH 让动态目标出现在 Home，但不允许收藏它们。收藏一个会消失的 Pod 没意义。该保存的是发现来源，不是 Pod 本身。

## 打开容器不是“伪 SSH”，而是 connector-backed PTY

动态发现目标最终产出的不是 Profile，而是 `connector_spec`。

Docker 容器对应：

```text
docker_exec
```

Kubernetes Pod 对应：

```text
kubectl_exec
```

这两个类型和 `profile`、`forward` 是平级概念。它们不是 SSH Profile 的变种，也不应该塞进 Profile 的字段里。

打开时，RSSH 在本机启动：

```bash
docker exec -it ...
kubectl exec -it ...
```

前端看到的仍然是一条 PTY 数据流。也就是说，终端层不需要关心下面是 SSH、local shell、Docker exec 还是 kubectl exec。

这是干净的边界：

- 发现层负责找到目标
- connector spec 描述如何进入目标
- PTY 层负责承载交互式终端
- Home 只负责展示可打开目标

没有把 Docker/Kubernetes 硬塞进 SSH Profile。没有把临时目标写进静态配置。没有为了一个平台污染通用模型。

## 为什么现在才需要这个

因为开发环境已经从“几台服务器”变成了“不断变化的运行时集合”。

过去：

```text
ssh devbox
ssh staging
ssh prod-bastion
```

现在：

```text
docker exec api-1
docker exec worker-7
kubectl exec deploy/api -n staging
kubectl exec pod/debug-shell -n prod
```

下一步还会是：

```text
aws ec2
aliyun ecs
cloudflare tunnel
```

如果 SSH 工具仍然只认识 host/port/credential，它就只能做连接管理。连接管理没有消失，但它已经不是全部。

现代开发工具需要回答的是：

> 当前环境里，有哪些东西现在可以进去？

动态发现就是这个问题的答案。

## RSSH 的取舍

这不是一个大而全的资源管理器。

RSSH 不做 Kubernetes Dashboard，不做 Docker Desktop，不做云厂商控制台。那些工具管理资源生命周期，RSSH 只关心一件事：

> 找到当前可进入的运行目标，然后打开一个可靠的终端。

所以它的边界很明确：

- 只检测本机 CLI 是否存在
- 只读取 CLI context
- 只发现运行中的容器 / Pod
- 发现结果不持久化
- 不在远端机器上做隐式探测
- 不把动态目标伪装成静态 Profile

这不是少做。是把数据结构放对地方。

## 怎么用

路径很短：

1. 打开设置里的“动态发现”
2. 新建来源
3. 选择 Docker CLI 或 kubectl CLI
4. 选择对应 context
5. 保存

回到 Home，RSSH 会把静态 Profile、Forward 和动态发现结果放在同一个列表里。你照常搜索，照常排序，照常打开。

区别只有一个：动态目标不会被收藏，也不会被写成 Profile。

如果 Docker 或 kubectl 没装，对应选项会置灰。RSSH 不会替你安装这些工具，也不会伪造 context。它只读取你本机已经配置好的 CLI 环境。

这点很重要。动态发现不是“平台接管”，而是“把你现有 CLI 工作流接进终端工作台”。

## 一个具体例子

假设你的机器上有两个 Docker context：

```text
desktop-linux
dev-remote
```

你在 RSSH 里新增一个 Docker 来源，选择 `dev-remote`。之后 Home 里出现的是这个 context 下当前正在运行的容器。

容器重启了，旧目标消失；新容器起来了，新目标出现。RSSH 不会把旧容器残留成死 Profile。

Kubernetes 也是同样的模型。

```text
context = staging
namespace = api
```

RSSH 发现的是 `staging/api` 下当前 Running 的 Pod。Pod 滚动发布后，Home 里的目标跟着变化。你不用删除旧配置，也不用手工新建一堆新配置。

这就是动态发现的价值：**配置稳定的是入口，变化的是结果。**

## 一句话

传统 SSH 工具还停在“维护服务器列表”的时代。

RSSH 的动态发现承认了现实：开发目标已经变成容器、Pod、context 和云资源组成的动态集合。

**静态连接继续用 Profile；动态目标实时发现，平级打开，用完即走。**

这才是现代 SSH 工具该有的模型。
