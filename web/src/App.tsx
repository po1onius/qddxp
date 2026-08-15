import { useCallback, useEffect, useMemo, useState } from 'react';
import type { FormEvent } from 'react';
import { CheckCircle2, ChevronLeft, ChevronRight, ClipboardCheck, CreditCard, RefreshCcw, Search, TriangleAlert, X } from 'lucide-react';
import QRCode from 'qrcode';
import { Link, Route, Routes, useNavigate, useParams } from 'react-router-dom';
import { AdminApp } from './AdminApp';
import { StoreBrand } from './StoreBrand';
import { useToast } from './Toast';
import {
  createOrder,
  getProduct,
  getStorefrontConfig,
  listOrdersByContact,
  listPaymentMethods,
  listProductPage,
  queryOrder,
} from './api/client';
import type { CreateOrderResult, OrderDetail, OrderSummary, PaymentMethod, Product, StorefrontConfig } from './types';

type PaymentReturnState =
  | { kind: 'none' }
  | { kind: 'success'; orderId: string }
  | { kind: 'error'; message: string; orderId: string };

const LAST_ORDER_ID_STORAGE = 'qddxp_last_order_id';
const LAST_CONTACT_STORAGE = 'qddxp_last_contact';
const PRODUCT_PAGE_SIZE = 20;
const CONTACT_ORDER_PAGE_SIZE = 20;
const CONTACT_MIN_LENGTH = 6;
const CONTACT_MAX_LENGTH = 50;
const ORDER_PASSWORD_MIN_LENGTH = 6;
const ORDER_PASSWORD_MAX_LENGTH = 50;
const ADMIN_PAGE_PATH = '/admin';
const ORDER_QUERY_PAGE_PATH = '/orders';
const ORDER_CREATE_PAGE_PATH = '/orders/new';

const currencyFormatter = new Intl.NumberFormat('zh-CN', {
  style: 'currency',
  currency: 'CNY',
});

function formatPrice(priceCents: number) {
  return currencyFormatter.format(priceCents / 100);
}

function imageBase64Src(imageBase64: string | null) {
  if (!imageBase64) {
    return null;
  }
  return imageBase64.startsWith('data:') ? imageBase64 : `data:image/png;base64,${imageBase64}`;
}

function getPaymentReturnState(): PaymentReturnState {
  const params = new URLSearchParams(window.location.search);
  const tradeStatus = params.get('trade_status');
  const hasPaymentReturn = params.has('trade_status') || params.has('out_trade_no') || params.has('trade_no');

  if (!hasPaymentReturn) {
    return { kind: 'none' };
  }

  const orderId = params.get('param')?.trim() ?? '';
  if (tradeStatus === 'TRADE_SUCCESS') {
    if (!orderId) {
      return { kind: 'error', message: '支付成功返回缺少订单参数，无法定位订单', orderId: '' };
    }
    return { kind: 'success', orderId };
  }

  return {
    kind: 'error',
    message: tradeStatus ? `支付未成功：${tradeStatus}` : '支付未成功',
    orderId,
  };
}

function getInitialSelectedOrderId(paymentReturn: PaymentReturnState) {
  // ePay 会把下单时的 param 原样带回。无论网关页面报告成功还是失败，只要订单号存在，
  // 都直接定位到该订单，让用户通过订单密码读取服务端保存的最终支付与交付状态。
  if (paymentReturn.kind !== 'none' && paymentReturn.orderId) {
    return paymentReturn.orderId;
  }

  const params = new URLSearchParams(window.location.search);
  return params.get('order_id') ?? sessionStorage.getItem(LAST_ORDER_ID_STORAGE) ?? '';
}

export function App() {
  const [storefront, setStorefront] = useState<StorefrontConfig | null>(null);
  const [storefrontError, setStorefrontError] = useState('');

  useEffect(() => {
    let cancelled = false;
    void getStorefrontConfig()
      .then((config) => {
        if (cancelled) {
          return;
        }
        setStorefront(config);
        document.title = config.shop_name;
      })
      .catch((error) => {
        if (cancelled) {
          return;
        }
        console.error('加载店铺展示配置失败', error);
        setStorefrontError(error instanceof Error ? error.message : '店铺展示配置加载失败');
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (storefrontError) {
    return (
      <main className="flex min-h-screen items-center justify-center bg-slate-50 px-4 text-slate-950">
        <section className="w-full max-w-md rounded-md border border-red-200 bg-red-50 p-6 text-red-800">
          <h1 className="text-lg font-semibold">店铺配置加载失败</h1>
          <p className="mt-2 break-words text-sm">{storefrontError}</p>
        </section>
      </main>
    );
  }
  if (!storefront) {
    return <main className="min-h-screen animate-pulse bg-slate-50" aria-label="正在加载店铺配置" />;
  }

  return (
    <Routes>
      <Route element={<ShopApp storefront={storefront} />} path="/" />
      <Route element={<OrdersApp storefront={storefront} />} path={ORDER_QUERY_PAGE_PATH} />
      <Route element={<CreateOrderApp storefront={storefront} />} path={`${ORDER_CREATE_PAGE_PATH}/:productId`} />
      <Route element={<AdminApp storefront={storefront} />} path={`${ADMIN_PAGE_PATH}/*`} />
      <Route element={<NotFoundPage storefront={storefront} />} path="*" />
    </Routes>
  );
}

function ShopApp({ storefront }: { storefront: StorefrontConfig }) {
  const { showToast } = useToast();
  const navigate = useNavigate();
  const [products, setProducts] = useState<Product[]>([]);
  const [productPage, setProductPage] = useState(1);
  const [productTotal, setProductTotal] = useState(0);
  const [detailsProduct, setDetailsProduct] = useState<Product | null>(null);
  const [loadingProducts, setLoadingProducts] = useState(true);

  useEffect(() => {
    void refreshProducts();
  }, []);

  async function refreshProducts() {
    await loadProductPage(productPage);
  }

  async function loadProductPage(page: number) {
    setLoadingProducts(true);

    try {
      const response = await listProductPage({ page, page_size: PRODUCT_PAGE_SIZE });
      setProducts(response.items);
      setProductPage(response.page);
      setProductTotal(response.total);
    } catch (error) {
      setProducts([]);
      setProductTotal(0);
      showToast({
        message: error instanceof Error ? error.message : '商品列表加载失败',
        type: 'error',
      });
    } finally {
      setLoadingProducts(false);
    }
  }

  function openCheckout(product: Product) {
    setDetailsProduct(null);
    navigate(`${ORDER_CREATE_PAGE_PATH}/${encodeURIComponent(product.id)}`);
  }

  return (
    <div className="min-h-screen bg-slate-50 text-slate-950">
      <header className="border-b border-slate-200 bg-white">
        <div className="mx-auto flex max-w-7xl flex-col gap-4 px-4 py-4 sm:flex-row sm:items-center sm:justify-between lg:px-8">
          <StoreBrand storefront={storefront} />
          <nav className="flex flex-wrap gap-2" aria-label="主导航">
            <Link
              className="inline-flex h-10 items-center gap-2 rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 transition hover:border-slate-500"
              to={ORDER_QUERY_PAGE_PATH}
            >
              <ClipboardCheck size={18} />
              订单查询
            </Link>
          </nav>
        </div>
      </header>

      <main className="mx-auto grid max-w-7xl gap-8 px-4 py-8 lg:grid-cols-[1fr_340px] lg:px-8">
        <section>
          <CatalogPage
            loading={loadingProducts}
            onCheckout={openCheckout}
            onOpenDetails={setDetailsProduct}
            onPageChange={(page) => void loadProductPage(page)}
            onRefresh={refreshProducts}
            page={productPage}
            pageSize={PRODUCT_PAGE_SIZE}
            products={products}
            total={productTotal}
          />
        </section>

        <aside className="space-y-4">
          <StatusPanel products={products} />
        </aside>
      </main>
      {detailsProduct && (
        <ProductDetailsModal
          onCheckout={openCheckout}
          onClose={() => setDetailsProduct(null)}
          product={detailsProduct}
        />
      )}
    </div>
  );
}

// 创建订单页面根据路由中的商品 ID 重新读取公开商品快照，支持刷新和直接访问。
function CreateOrderApp({ storefront }: { storefront: StorefrontConfig }) {
  const { productId } = useParams<{ productId: string }>();
  const navigate = useNavigate();
  const { showToast } = useToast();
  const [product, setProduct] = useState<Product | null>(null);
  const [paymentMethods, setPaymentMethods] = useState<PaymentMethod[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState('');

  useEffect(() => {
    let cancelled = false;

    async function loadOrderCreationContext() {
      if (!productId) {
        setLoadError('下单地址缺少商品 ID');
        setLoading(false);
        return;
      }

      setLoading(true);
      setLoadError('');
      const [productResult, paymentMethodsResult] = await Promise.allSettled([
        getProduct(productId),
        listPaymentMethods(),
      ]);
      if (cancelled) {
        return;
      }

      if (productResult.status === 'fulfilled') {
        setProduct(productResult.value);
      } else {
        console.error('加载下单商品失败', { productId, error: productResult.reason });
        setProduct(null);
        setLoadError(productResult.reason instanceof Error ? productResult.reason.message : '商品加载失败');
      }

      if (paymentMethodsResult.status === 'fulfilled') {
        setPaymentMethods(paymentMethodsResult.value);
      } else {
        console.error('加载支付方式失败', { productId, error: paymentMethodsResult.reason });
        setPaymentMethods([]);
        showToast({
          message: paymentMethodsResult.reason instanceof Error ? paymentMethodsResult.reason.message : '支付方式加载失败',
          type: 'error',
        });
      }
      setLoading(false);
    }

    void loadOrderCreationContext();
    return () => {
      cancelled = true;
    };
  }, [productId, showToast]);

  // 保持回调引用稳定，避免二维码轮询因为父组件重渲染而反复重启。
  const handleCreated = useCallback(
    (order: CreateOrderResult) => {
      sessionStorage.setItem(LAST_ORDER_ID_STORAGE, order.id);
      if (order.payment_action?.type === 'redirect') {
        window.location.href = order.payment_action.url;
        return;
      }
      navigate(`${ORDER_QUERY_PAGE_PATH}?order_id=${encodeURIComponent(order.id)}`);
    },
    [navigate],
  );

  return (
    <div className="min-h-screen bg-slate-50 text-slate-950">
      <header className="border-b border-slate-200 bg-white">
        <div className="mx-auto flex max-w-7xl flex-col gap-4 px-4 py-4 sm:flex-row sm:items-center sm:justify-between lg:px-8">
          <StoreBrand storefront={storefront} />
          <nav aria-label="创建订单导航">
            <Link
              className="inline-flex h-10 items-center gap-2 rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 transition hover:border-slate-500"
              to="/"
            >
              <ChevronLeft size={18} />
              返回商城
            </Link>
          </nav>
        </div>
      </header>
      <main className="mx-auto max-w-7xl px-4 py-8 lg:px-8">
        {/* 页面标题属于当前业务内容，不放入全站复用的品牌区域。 */}
        <h1 className="mb-6 text-2xl font-semibold">创建订单</h1>
        {loading && (
          <div className="h-96 max-w-2xl animate-pulse rounded-md border border-slate-200 bg-white" aria-label="正在加载下单页面" />
        )}
        {!loading && loadError && (
          <section className="max-w-2xl rounded-md border border-red-200 bg-red-50 p-6 text-red-800">
            <h2 className="text-lg font-semibold">无法创建订单</h2>
            <p className="mt-2 text-sm">{loadError}</p>
            <Link className="mt-5 inline-flex h-10 items-center rounded-md bg-slate-950 px-4 text-sm font-medium text-white" to="/">
              返回商品列表
            </Link>
          </section>
        )}
        {!loading && product && (
          <CheckoutPage
            onBack={() => navigate('/')}
            onCreated={handleCreated}
            paymentMethods={paymentMethods}
            product={product}
          />
        )}
      </main>
    </div>
  );
}

// 订单查询使用独立页面组件，进入该路由时不会加载商品目录和支付方式。
function OrdersApp({ storefront }: { storefront: StorefrontConfig }) {
  return (
    <div className="min-h-screen bg-slate-50 text-slate-950">
      <header className="border-b border-slate-200 bg-white">
        <div className="mx-auto flex max-w-7xl flex-col gap-4 px-4 py-4 sm:flex-row sm:items-center sm:justify-between lg:px-8">
          <StoreBrand storefront={storefront} />
          <nav aria-label="订单查询导航">
            <Link
              className="inline-flex h-10 items-center gap-2 rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 transition hover:border-slate-500"
              to="/"
            >
              <ChevronLeft size={18} />
              返回商城
            </Link>
          </nav>
        </div>
      </header>
      <main className="mx-auto max-w-7xl px-4 py-8 lg:px-8">
        <h1 className="mb-6 text-2xl font-semibold">订单查询</h1>
        <OrderQueryPage />
      </main>
    </div>
  );
}

function NotFoundPage({ storefront }: { storefront: StorefrontConfig }) {
  return (
    <div className="min-h-screen bg-slate-50 text-slate-950">
      <header className="border-b border-slate-200 bg-white">
        <div className="mx-auto max-w-7xl px-4 py-4 lg:px-8">
          <StoreBrand storefront={storefront} />
        </div>
      </header>
      <main className="flex items-center justify-center px-4 py-16">
        <section className="w-full max-w-md rounded-md border border-slate-200 bg-white p-6 text-center shadow-panel">
          <h1 className="text-xl font-semibold">页面不存在</h1>
          <p className="mt-2 text-sm text-slate-500">请检查访问地址，或返回商城继续浏览。</p>
          <Link
            className="mt-5 inline-flex h-10 items-center justify-center rounded-md bg-slate-950 px-4 text-sm font-medium text-white"
            to="/"
          >
            返回商城
          </Link>
        </section>
      </main>
    </div>
  );
}

function CatalogPage({
  loading,
  onCheckout,
  onOpenDetails,
  onPageChange,
  onRefresh,
  page,
  pageSize,
  products,
  total,
}: {
  loading: boolean;
  onCheckout: (product: Product) => void;
  onOpenDetails: (product: Product) => void;
  onPageChange: (page: number) => void;
  onRefresh: () => Promise<void>;
  page: number;
  pageSize: number;
  products: Product[];
  total: number;
}) {
  const totalPages = Math.max(1, Math.ceil(total / pageSize));

  return (
    <div className="space-y-5">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h1 className="text-2xl font-semibold">商品列表</h1>
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            className="inline-flex h-10 items-center justify-center rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500 disabled:cursor-not-allowed disabled:opacity-50"
            disabled={loading || page <= 1}
            onClick={() => onPageChange(page - 1)}
            type="button"
          >
            上一页
          </button>
          <span className="inline-flex h-10 items-center rounded-md border border-slate-200 bg-white px-3 text-sm text-slate-600">
            {page} / {totalPages}
          </span>
          <button
            className="inline-flex h-10 items-center justify-center rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500 disabled:cursor-not-allowed disabled:opacity-50"
            disabled={loading || page >= totalPages}
            onClick={() => onPageChange(page + 1)}
            type="button"
          >
            下一页
          </button>
          <button
            className="inline-flex h-10 items-center justify-center gap-2 rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500"
            onClick={() => void onRefresh()}
            type="button"
          >
            <RefreshCcw size={18} />
            刷新
          </button>
        </div>
      </div>

      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
        {(loading ? Array.from({ length: 6 }) : products).map((product, index) =>
          loading ? (
            <div key={index} className="h-[336px] animate-pulse rounded-md border border-slate-200 bg-white" />
          ) : (
            <ProductCard
              key={(product as Product).id}
              product={product as Product}
              onCheckout={onCheckout}
              onOpenDetails={onOpenDetails}
            />
          ),
        )}
      </div>
    </div>
  );
}

function ProductCard({
  onCheckout,
  onOpenDetails,
  product,
}: {
  onCheckout: (product: Product) => void;
  onOpenDetails: (product: Product) => void;
  product: Product;
}) {
  const soldOut = product.stock <= 0;
  const imageSrc = imageBase64Src(product.image_base64);

  return (
    <article className="overflow-hidden rounded-md border border-slate-200 bg-white shadow-panel">
      <button className="block w-full text-left" onClick={() => onOpenDetails(product)} type="button">
        <div className="flex h-48 items-center justify-center bg-slate-100">
          {imageSrc ? (
            <img alt={product.name} className="h-full w-full object-contain p-2" src={imageSrc} />
          ) : (
            <div className="flex h-full items-center justify-center bg-slate-200 text-sm font-medium text-slate-500">{product.name}</div>
          )}
        </div>
        <div className="p-4 pb-3">
          <div className="flex items-start justify-between gap-3">
            <div>
              <h3 className="text-base font-semibold leading-6">{product.name}</h3>
              <p className="mt-1 text-sm text-slate-500">库存 {product.stock} · 已售 {product.sold_count}</p>
            </div>
            <p className="shrink-0 text-base font-semibold text-emerald-700">{formatPrice(product.price_cents)}</p>
          </div>
        </div>
      </button>
      <div className="px-4 pb-4">
        <button
          className="inline-flex h-10 w-full items-center justify-center gap-2 rounded-md bg-slate-950 px-3 text-sm font-medium text-white hover:bg-slate-800 disabled:cursor-not-allowed disabled:bg-slate-300"
          disabled={soldOut}
          onClick={() => onCheckout(product)}
          type="button"
        >
          <CreditCard size={18} />
          {soldOut ? '暂时无货' : '购买'}
        </button>
      </div>
    </article>
  );
}

function ProductDetailsModal({
  onCheckout,
  onClose,
  product,
}: {
  onCheckout: (product: Product) => void;
  onClose: () => void;
  product: Product;
}) {
  const soldOut = product.stock <= 0;
  const imageSrc = imageBase64Src(product.image_base64);

  return (
    <div className="fixed inset-0 z-40 flex items-end justify-center bg-slate-950/45 px-4 py-4 sm:items-center">
      <section className="max-h-[calc(100vh-2rem)] w-full max-w-2xl overflow-hidden rounded-md bg-white shadow-panel">
        <div className="flex items-start justify-between gap-4 border-b border-slate-200 px-5 py-4">
          <div>
            <h2 className="text-lg font-semibold">{product.name}</h2>
            <p className="mt-1 text-sm text-slate-500">库存 {product.stock} · 已售 {product.sold_count}</p>
          </div>
          <button
            aria-label="关闭商品详情"
            className="rounded-md p-1 text-slate-500 hover:bg-slate-100 hover:text-slate-900"
            onClick={onClose}
            type="button"
          >
            <X size={20} />
          </button>
        </div>
        <div className="max-h-[calc(100vh-11rem)] overflow-auto">
          {imageSrc && (
            <div className="flex h-72 items-center justify-center bg-slate-100 sm:h-80">
              <img alt={product.name} className="h-full w-full object-contain p-3" src={imageSrc} />
            </div>
          )}
          <div className="space-y-5 p-5">
            <div className="flex items-center justify-between gap-4">
              <span className="text-sm text-slate-500">价格</span>
              <span className="text-lg font-semibold text-emerald-700">{formatPrice(product.price_cents)}</span>
            </div>
            <div>
              <p className="text-sm font-medium text-slate-700">商品详情</p>
              <p className="mt-2 whitespace-pre-wrap break-words text-sm leading-6 text-slate-600">
                {product.details || '暂无详情'}
              </p>
            </div>
          </div>
        </div>
        <div className="border-t border-slate-200 p-5">
          <button
            className="inline-flex h-10 w-full items-center justify-center gap-2 rounded-md bg-slate-950 px-3 text-sm font-medium text-white hover:bg-slate-800 disabled:cursor-not-allowed disabled:bg-slate-300"
            disabled={soldOut}
            onClick={() => onCheckout(product)}
            type="button"
          >
            <CreditCard size={18} />
            {soldOut ? '暂时无货' : '购买'}
          </button>
        </div>
      </section>
    </div>
  );
}

function ProductDetailsBlock({ product }: { product: Product }) {
  return (
    <div className="rounded-md border border-slate-200 bg-slate-50 p-4">
      <p className="text-sm font-medium text-slate-700">商品详情</p>
      <p className="mt-2 whitespace-pre-wrap break-words text-sm leading-6 text-slate-600">{product.details || '暂无详情'}</p>
    </div>
  );
}

function CheckoutPage({
  onBack,
  onCreated,
  paymentMethods,
  product,
}: {
  onBack: () => void;
  onCreated: (order: CreateOrderResult) => void;
  paymentMethods: PaymentMethod[];
  product: Product;
}) {
  const { showToast } = useToast();
  const [contact, setContact] = useState('');
  const [orderPassword, setOrderPassword] = useState('');
  const [selectedPayment, setSelectedPayment] = useState<PaymentMethod | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [qrOrder, setQrOrder] = useState<CreateOrderResult | null>(null);
  const [qrImage, setQrImage] = useState('');
  const [remainingSeconds, setRemainingSeconds] = useState(0);

  useEffect(() => {
    if (!selectedPayment && paymentMethods.length > 0) {
      setSelectedPayment(paymentMethods[0]);
    } else if (
      selectedPayment &&
      !paymentMethods.some(
        (method) => method.provider === selectedPayment.provider && method.channel === selectedPayment.channel,
      )
    ) {
      setSelectedPayment(paymentMethods[0] ?? null);
    }
  }, [paymentMethods, selectedPayment]);

  useEffect(() => {
    const action = qrOrder?.payment_action;
    if (!qrOrder || action?.type !== 'qr_code') {
      setQrImage('');
      setRemainingSeconds(0);
      return;
    }

    let stopped = false;
    void QRCode.toDataURL(action.content, {
      errorCorrectionLevel: 'M',
      margin: 2,
      width: 280,
    })
      .then((image) => {
        if (!stopped) {
          setQrImage(image);
        }
      })
      .catch(() => {
        if (!stopped) {
          showToast({ message: '支付二维码生成失败', type: 'error' });
        }
      });

    const expiresAt = new Date(action.expires_at).getTime();
    let countdown: number | undefined;
    let poll: number | undefined;

    // 服务端返回的支付截止时间是自动轮询的硬边界。倒计时归零后同时清理两个定时器，
    // 页面即使长期停留也不会继续产生已经没有意义的订单查询请求。
    const stopPolling = () => {
      stopped = true;
      if (countdown !== undefined) {
        window.clearInterval(countdown);
      }
      if (poll !== undefined) {
        window.clearInterval(poll);
      }
    };
    const updateRemaining = () => {
      const seconds = Math.max(0, Math.ceil((expiresAt - Date.now()) / 1000));
      setRemainingSeconds(seconds);
      if (seconds === 0) {
        stopPolling();
      }
    };
    updateRemaining();
    if (stopped) {
      return stopPolling;
    }

    countdown = window.setInterval(updateRemaining, 1000);
    poll = window.setInterval(async () => {
      // 定时器触发点可能刚好跨过截止时间，因此请求发出前再检查一次，避免等待下一次
      // 一秒倒计时才停止。
      if (Date.now() >= expiresAt) {
        setRemainingSeconds(0);
        stopPolling();
        return;
      }
      try {
        const detail = await queryOrder({ id: qrOrder.id, order_password: orderPassword });
        if (!stopped && detail.status === 'delivered') {
          stopPolling();
          showToast({ message: '微信支付已确认，正在进入订单详情', type: 'success' });
          onCreated(qrOrder);
        }
      } catch {
        // 轮询失败通常是瞬时网络问题；保留二维码并等待下一轮，不用连续打扰用户。
      }
    }, 2500);

    return () => {
      stopPolling();
    };
  }, [onCreated, orderPassword, qrOrder, showToast]);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const contactLength = Array.from(contact.trim()).length;
    if (contactLength < CONTACT_MIN_LENGTH) {
      showToast({ message: `联系方式不能少于 ${CONTACT_MIN_LENGTH} 个字符`, type: 'error' });
      return;
    }
    if (contactLength > CONTACT_MAX_LENGTH) {
      showToast({ message: `联系方式不能超过 ${CONTACT_MAX_LENGTH} 个字符`, type: 'error' });
      return;
    }
    const orderPasswordLength = Array.from(orderPassword).length;
    if (orderPasswordLength < ORDER_PASSWORD_MIN_LENGTH) {
      showToast({ message: `订单密码不能少于 ${ORDER_PASSWORD_MIN_LENGTH} 个字符`, type: 'error' });
      return;
    }
    if (orderPasswordLength > ORDER_PASSWORD_MAX_LENGTH) {
      showToast({ message: `订单密码不能超过 ${ORDER_PASSWORD_MAX_LENGTH} 个字符`, type: 'error' });
      return;
    }
    if (product.stock <= 0) {
      showToast({ message: '商品暂时无货，请返回商品列表', type: 'error' });
      return;
    }
    if (!selectedPayment) {
      showToast({ message: '当前没有可用的支付方式', type: 'error' });
      return;
    }

    setSubmitting(true);

    try {
      const order = await createOrder({
        product_info_id: product.id,
        contact,
        order_password: orderPassword,
        payment: {
          provider: selectedPayment.provider,
          channel: selectedPayment.channel,
        },
      });
      sessionStorage.setItem(LAST_CONTACT_STORAGE, contact.trim());
      sessionStorage.setItem(LAST_ORDER_ID_STORAGE, order.id);
      if (order.payment_error) {
        showToast({ message: order.payment_error, type: 'error' });
      }
      if (order.payment_action?.type === 'qr_code') {
        setQrOrder(order);
      } else {
        onCreated(order);
      }
    } catch (err) {
      showToast({
        message: err instanceof Error ? err.message : '创建订单失败',
        type: 'error',
      });
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <form className="max-w-2xl space-y-5 rounded-md border border-slate-200 bg-white p-6 shadow-panel" onSubmit={submit}>
      <p className="text-sm text-slate-500">订单密码用于支付后查询和取货，请自行保存。</p>

      <div className="rounded-md border border-slate-200 bg-slate-50 p-4">
        <p className="text-sm text-slate-500">当前商品</p>
        <div className="flex items-start justify-between gap-3">
          <div>
            <p className="font-semibold">{product.name}</p>
          </div>
          <p className="font-semibold text-emerald-700">{formatPrice(product.price_cents)}</p>
        </div>
      </div>

      <ProductDetailsBlock product={product} />

      <label className="block">
        <span className="text-sm font-medium text-slate-700">联系方式</span>
        <input
          className="mt-2 h-11 w-full rounded-md border border-slate-300 px-3 text-sm outline-none focus:border-slate-900"
          maxLength={CONTACT_MAX_LENGTH}
          minLength={CONTACT_MIN_LENGTH}
          onChange={(event) => setContact(event.target.value)}
          placeholder="邮箱、QQ 或手机号"
          required
          value={contact}
        />
      </label>

      <label className="block">
        <span className="text-sm font-medium text-slate-700">订单密码</span>
        <input
          className="mt-2 h-11 w-full rounded-md border border-slate-300 px-3 text-sm outline-none focus:border-slate-900"
          maxLength={ORDER_PASSWORD_MAX_LENGTH}
          minLength={ORDER_PASSWORD_MIN_LENGTH}
          onChange={(event) => setOrderPassword(event.target.value)}
          placeholder="6～50 位"
          required
          type="password"
          value={orderPassword}
        />
      </label>

      <fieldset>
        <legend className="text-sm font-medium text-slate-700">支付方式</legend>
        <div className="mt-2 grid grid-cols-2 gap-2">
          {paymentMethods.map((method) => (
            <button
              className={`h-10 rounded-md border px-3 text-sm font-medium ${
                selectedPayment?.provider === method.provider && selectedPayment.channel === method.channel
                  ? 'border-slate-950 bg-slate-950 text-white'
                  : 'border-slate-300 bg-white text-slate-700'
              }`}
              key={`${method.provider}/${method.channel}`}
              onClick={() => setSelectedPayment(method)}
              type="button"
            >
              {method.label}
            </button>
          ))}
        </div>
        {paymentMethods.length === 0 && <p className="mt-2 text-sm text-amber-700">管理员尚未配置支付方式。</p>}
      </fieldset>

      <div className="flex flex-col-reverse gap-3 sm:flex-row sm:justify-end">
        <button className="h-10 rounded-md border border-slate-300 px-4 text-sm font-medium text-slate-700" onClick={onBack} type="button">
          返回
        </button>
        <button
          className="inline-flex h-10 items-center justify-center gap-2 rounded-md bg-slate-950 px-4 text-sm font-medium text-white disabled:cursor-wait disabled:bg-slate-400"
          disabled={submitting || !selectedPayment || product.stock <= 0}
          type="submit"
        >
          <CreditCard size={18} />
          {product.stock <= 0 ? '暂时无货' : submitting ? '提交中' : '提交并支付'}
        </button>
      </div>
      {qrOrder?.payment_action?.type === 'qr_code' && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/60 p-4" role="dialog" aria-modal="true">
          <div className="w-full max-w-sm rounded-lg bg-white p-6 text-center shadow-xl">
            <div className="flex items-start justify-between gap-3 text-left">
              <div>
                <h3 className="text-lg font-semibold">微信扫码支付</h3>
                <p className="mt-1 text-sm text-slate-500">订单号：{qrOrder.id}</p>
              </div>
              <button
                aria-label="关闭支付二维码"
                className="rounded p-1 text-slate-500 hover:bg-slate-100"
                onClick={() => onCreated(qrOrder)}
                type="button"
              >
                <X size={20} />
              </button>
            </div>
            <div className="mt-5 flex min-h-72 items-center justify-center rounded-md border border-slate-200 bg-white p-3">
              {qrImage ? <img alt="微信支付二维码" className="h-64 w-64" src={qrImage} /> : <p className="text-sm text-slate-500">二维码生成中...</p>}
            </div>
            <p className={`mt-3 text-sm ${remainingSeconds > 0 ? 'text-slate-600' : 'text-red-600'}`}>
              {remainingSeconds > 0
                ? `请在 ${Math.floor(remainingSeconds / 60)}:${String(remainingSeconds % 60).padStart(2, '0')} 内支付`
                : '支付二维码已过期，请重新下单'}
            </p>
            <p className="mt-3 text-xs leading-5 text-slate-500">支付确认后页面会自动进入订单查询；关闭窗口不会取消订单。</p>
          </div>
        </div>
      )}
    </form>
  );
}

function OrderQueryPage() {
  const { showToast } = useToast();
  const navigate = useNavigate();
  const [paymentReturn] = useState(() => getPaymentReturnState());
  const [contact, setContact] = useState(() => sessionStorage.getItem(LAST_CONTACT_STORAGE) ?? '');
  const [searchedContact, setSearchedContact] = useState('');
  const [orders, setOrders] = useState<OrderSummary[]>([]);
  const [ordersPage, setOrdersPage] = useState(1);
  const [ordersTotal, setOrdersTotal] = useState(0);
  const [selectedOrderId, setSelectedOrderId] = useState(() => getInitialSelectedOrderId(paymentReturn));
  const [orderPassword, setOrderPassword] = useState('');
  const [order, setOrder] = useState<OrderDetail | null>(null);
  const [loadingOrders, setLoadingOrders] = useState(false);
  const [loadingDetail, setLoadingDetail] = useState(false);
  const selectedOrder = orders.find((item) => item.id === selectedOrderId) ?? null;
  const canQuerySelectedOrder = selectedOrder
    ? selectedOrder.status === 'delivered' ||
      isPaymentReceivedAfterExpiry(selectedOrder.status, selectedOrder.payment_paid_at)
    : Boolean(selectedOrderId);
  const selectedOrderTitle = selectedOrder?.product_name ?? '当前订单';
  const selectedOrderMeta = selectedOrder
    ? `订单状态：${statusText(selectedOrder.status, selectedOrder.payment_paid_at)}`
    : '订单状态：请输入订单密码查询';
  const ordersTotalPages = Math.max(1, Math.ceil(ordersTotal / CONTACT_ORDER_PAGE_SIZE));

  useEffect(() => {
    if (paymentReturn.kind === 'none' || !paymentReturn.orderId) {
      return;
    }

    // ePay 回跳会附带 trade_no、签名等网关参数。识别出透传订单号后立即替换为与微信
    // Native 支付一致的具体订单地址，避免刷新或复制链接时继续传播支付网关参数。
    const orderPath = `${ORDER_QUERY_PAGE_PATH}?order_id=${encodeURIComponent(paymentReturn.orderId)}`;
    if (`${window.location.pathname}${window.location.search}` === orderPath) {
      return;
    }
    console.info('ePay 回跳已归一化为具体订单查询地址', { orderId: paymentReturn.orderId });
    navigate(orderPath, { replace: true });
  }, [navigate, paymentReturn]);

  useEffect(() => {
    if (selectedOrderId) {
      sessionStorage.setItem(LAST_ORDER_ID_STORAGE, selectedOrderId);
    }
  }, [selectedOrderId]);

  async function searchOrders(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmedContact = contact.trim();
    if (!trimmedContact) {
      showToast({ message: '请输入下单时填写的联系方式', type: 'error' });
      return;
    }
    const contactLength = Array.from(trimmedContact).length;
    if (contactLength < CONTACT_MIN_LENGTH) {
      showToast({ message: `联系方式不能少于 ${CONTACT_MIN_LENGTH} 个字符`, type: 'error' });
      return;
    }
    if (contactLength > CONTACT_MAX_LENGTH) {
      showToast({ message: `联系方式不能超过 ${CONTACT_MAX_LENGTH} 个字符`, type: 'error' });
      return;
    }

    setLoadingOrders(true);
    setOrders([]);
    setOrdersPage(1);
    setOrdersTotal(0);
    setOrder(null);
    setOrderPassword('');

    try {
      const response = await listOrdersByContact({
        contact: trimmedContact,
        page: 1,
        page_size: CONTACT_ORDER_PAGE_SIZE,
      });
      const foundOrders = response.items;
      setOrders(foundOrders);
      setOrdersPage(response.page);
      setOrdersTotal(response.total);
      setSearchedContact(trimmedContact);
      sessionStorage.setItem(LAST_CONTACT_STORAGE, trimmedContact);

      if (selectedOrderId && foundOrders.some((item) => item.id === selectedOrderId)) {
        return;
      }
      setSelectedOrderId(foundOrders.length === 1 ? foundOrders[0].id : '');
    } catch (err) {
      showToast({
        message: err instanceof Error ? err.message : '查询订单列表失败',
        type: 'error',
      });
    } finally {
      setLoadingOrders(false);
    }
  }

  async function loadOrderPage(trimmedContact: string, page: number, errorMessage: string) {
    setLoadingOrders(true);

    try {
      const response = await listOrdersByContact({
        contact: trimmedContact,
        page,
        page_size: CONTACT_ORDER_PAGE_SIZE,
      });
      const foundOrders = response.items;
      setOrders(foundOrders);
      setOrdersPage(response.page);
      setOrdersTotal(response.total);

      if (!foundOrders.some((item) => item.id === selectedOrderId)) {
        setSelectedOrderId(foundOrders.length === 1 ? foundOrders[0].id : '');
        setOrder(null);
        setOrderPassword('');
      }
    } catch (err) {
      showToast({
        message: err instanceof Error ? err.message : errorMessage,
        type: 'error',
      });
    } finally {
      setLoadingOrders(false);
    }
  }

  async function loadNextOrdersPage() {
    const trimmedContact = searchedContact.trim();
    if (!trimmedContact || ordersPage >= ordersTotalPages) {
      return;
    }

    await loadOrderPage(trimmedContact, ordersPage + 1, '订单翻页失败');
  }

  async function loadPreviousOrdersPage() {
    const trimmedContact = searchedContact.trim();
    if (!trimmedContact || ordersPage <= 1) {
      return;
    }

    await loadOrderPage(trimmedContact, ordersPage - 1, '订单翻页失败');
  }

  async function querySelectedOrder(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!selectedOrderId) {
      showToast({ message: '请选择订单', type: 'error' });
      return;
    }
    const orderPasswordLength = Array.from(orderPassword).length;
    if (orderPasswordLength < ORDER_PASSWORD_MIN_LENGTH) {
      showToast({ message: `订单密码不能少于 ${ORDER_PASSWORD_MIN_LENGTH} 个字符`, type: 'error' });
      return;
    }
    if (orderPasswordLength > ORDER_PASSWORD_MAX_LENGTH) {
      showToast({ message: `订单密码不能超过 ${ORDER_PASSWORD_MAX_LENGTH} 个字符`, type: 'error' });
      return;
    }

    setLoadingDetail(true);
    setOrder(null);

    try {
      setOrder(await queryOrder({ id: selectedOrderId, order_password: orderPassword }));
    } catch (err) {
      showToast({
        message: err instanceof Error ? err.message : '查询订单失败',
        type: 'error',
      });
    } finally {
      setLoadingDetail(false);
    }
  }

  return (
    <div className="max-w-3xl space-y-5">
      {paymentReturn.kind === 'error' && <PaymentReturnNotice message={paymentReturn.message} type="error" />}
      {paymentReturn.kind === 'success' && (
        <PaymentReturnNotice message="已从支付页面返回，请输入订单密码查询支付与发货结果" type="success" />
      )}

      {paymentReturn.kind !== 'success' && (
        <form className="space-y-5 rounded-md border border-slate-200 bg-white p-6 shadow-panel" onSubmit={searchOrders}>
          <p className="text-sm text-slate-500">使用下单时填写的联系方式查找订单。</p>
          <label className="block">
            <span className="text-sm font-medium text-slate-700">联系方式</span>
            <input
              className="mt-2 h-11 w-full rounded-md border border-slate-300 px-3 text-sm outline-none focus:border-slate-900"
              maxLength={CONTACT_MAX_LENGTH}
              minLength={CONTACT_MIN_LENGTH}
              onChange={(event) => setContact(event.target.value)}
              placeholder="邮箱、QQ 或手机号"
              required
              value={contact}
            />
          </label>
          <button
            className="inline-flex h-10 items-center justify-center gap-2 rounded-md bg-slate-950 px-4 text-sm font-medium text-white disabled:cursor-wait disabled:bg-slate-400"
            disabled={loadingOrders}
            type="submit"
          >
            <Search size={18} />
            {loadingOrders ? '查询中' : '查询订单'}
          </button>
        </form>
      )}

      {orders.length > 0 && (
        <section className="rounded-md border border-slate-200 bg-white p-6 shadow-panel">
          <div className="flex items-center justify-between gap-3">
            <h3 className="text-base font-semibold">订单列表</h3>
            <span className="text-sm text-slate-500">
              第 {ordersPage} / {ordersTotalPages} 页 · 当前 {orders.length} 个订单 · 共 {ordersTotal} 个
            </span>
          </div>
          <div className="mt-4 grid gap-3">
            {orders.map((item) => (
              <button
                className={`rounded-md border p-4 text-left transition ${
                  selectedOrderId === item.id
                    ? 'border-slate-950 bg-slate-50'
                    : 'border-slate-200 bg-white hover:border-slate-400'
                }`}
                key={item.id}
                onClick={() => {
                  setSelectedOrderId(item.id);
                  setOrder(null);
                  setOrderPassword('');
                }}
                type="button"
              >
                <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                  <div className="min-w-0">
                    <p className="truncate font-medium">{item.product_name}</p>
                    <p className="mt-1 text-sm text-slate-500">{formatPrice(item.price_cents)}</p>
                  </div>
                  <div className="flex shrink-0 flex-col items-start gap-2 sm:items-end">
                    <OrderStatusBadge paymentPaidAt={item.payment_paid_at} status={item.status} />
                    <span className="text-xs text-slate-500">{formatDate(item.created_at)}</span>
                  </div>
                </div>
                <p className="mt-3 break-all font-mono text-xs text-slate-500">{item.id}</p>
              </button>
            ))}
          </div>
          <OrderListPaginationControls
            loading={loadingOrders}
            onNextPage={() => void loadNextOrdersPage()}
            onPreviousPage={() => void loadPreviousOrdersPage()}
            page={ordersPage}
            totalPages={ordersTotalPages}
          />
        </section>
      )}

      {!loadingOrders && searchedContact && searchedContact === contact.trim() && orders.length === 0 && (
        <p className="rounded-md border border-slate-200 bg-white px-4 py-3 text-sm text-slate-500">暂无匹配订单</p>
      )}

      {canQuerySelectedOrder && (
        <form className="space-y-5 rounded-md border border-slate-200 bg-white p-6 shadow-panel" onSubmit={querySelectedOrder}>
          <div>
            <h3 className="text-base font-semibold">查看发货内容</h3>
            <p className="mt-1 text-sm text-slate-500">{selectedOrderTitle}</p>
          </div>
          <div className="rounded-md border border-slate-200 bg-slate-50 p-4">
            <p className="break-all font-mono text-xs text-slate-500">{selectedOrderId}</p>
            <p className="mt-2 text-sm text-slate-600">{selectedOrderMeta}</p>
          </div>
          <label className="block">
            <span className="text-sm font-medium text-slate-700">订单密码</span>
            <input
              className="mt-2 h-11 w-full rounded-md border border-slate-300 px-3 text-sm outline-none focus:border-slate-900"
              maxLength={ORDER_PASSWORD_MAX_LENGTH}
              minLength={ORDER_PASSWORD_MIN_LENGTH}
              onChange={(event) => setOrderPassword(event.target.value)}
              required
              type="password"
              value={orderPassword}
            />
          </label>
          <button
            className="inline-flex h-10 items-center justify-center gap-2 rounded-md bg-slate-950 px-4 text-sm font-medium text-white disabled:cursor-wait disabled:bg-slate-400"
            disabled={loadingDetail}
            type="submit"
          >
            <Search size={18} />
            {loadingDetail ? '查询中' : '查看发货'}
          </button>
        </form>
      )}

      {order && <OrderResult order={order} />}
    </div>
  );
}

function OrderListPaginationControls({
  loading,
  onNextPage,
  onPreviousPage,
  page,
  totalPages,
}: {
  loading: boolean;
  onNextPage: () => void;
  onPreviousPage: () => void;
  page: number;
  totalPages: number;
}) {
  return (
    <div className="mt-4 flex flex-wrap gap-2 border-t border-slate-200 pt-4">
      <button
        className="inline-flex h-10 items-center justify-center gap-1 rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500 disabled:cursor-not-allowed disabled:opacity-50"
        disabled={loading || page <= 1}
        onClick={onPreviousPage}
        type="button"
      >
        <ChevronLeft size={18} />
        上一页
      </button>
      <button
        className="inline-flex h-10 items-center justify-center gap-1 rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500 disabled:cursor-not-allowed disabled:opacity-50"
        disabled={loading || page >= totalPages}
        onClick={onNextPage}
        type="button"
      >
        下一页
        <ChevronRight size={18} />
      </button>
    </div>
  );
}

function PaymentReturnNotice({ message, type }: { message: string; type: 'success' | 'error' }) {
  const className = type === 'success' ? 'border-emerald-200 bg-emerald-50 text-emerald-800' : 'border-red-200 bg-red-50 text-red-700';

  return <p className={`rounded-md border px-4 py-3 text-sm font-medium ${className}`}>{message}</p>;
}

function OrderResult({ order }: { order: OrderDetail }) {
  const paymentReceivedAfterExpiry = isPaymentReceivedAfterExpiry(order.status, order.payment_paid_at);
  if (paymentReceivedAfterExpiry) {
    return (
      <section className="rounded-md border border-red-200 bg-red-50 p-6 shadow-panel">
        <div className="flex items-start gap-3 text-red-800">
          <TriangleAlert className="shrink-0" size={22} />
          <div>
            <h3 className="text-base font-semibold">{order.product_name}</h3>
            <p className="mt-2 text-sm font-medium">异常订单，联系管理员处理</p>
            <p className="mt-2 text-xs text-red-700">订单超时释放库存后才收到支付通知，系统不会自动分配库存或发货。</p>
          </div>
        </div>
      </section>
    );
  }

  const delivered = order.status === 'delivered' && Boolean(order.content);
  const contentText = order.content ?? '支付确认后显示';

  return (
    <section className="rounded-md border border-slate-200 bg-white p-6 shadow-panel">
      <div className="flex items-start gap-3">
        <CheckCircle2 className={delivered ? 'text-emerald-600' : 'text-amber-600'} size={22} />
        <div>
          <h3 className="text-base font-semibold">{order.product_name}</h3>
          <p className="mt-1 text-sm text-slate-500">订单状态：{statusText(order.status, order.payment_paid_at)}</p>
        </div>
      </div>
      <div className="mt-5 rounded-md border border-slate-200 bg-slate-50 p-4">
        <p className="text-sm font-medium text-slate-700">发货内容</p>
        <pre className="mt-2 whitespace-pre-wrap break-words text-sm text-slate-700">{contentText}</pre>
      </div>
    </section>
  );
}

function OrderStatusBadge({ paymentPaidAt, status }: { paymentPaidAt: string | null; status: string }) {
  const delivered = status === 'delivered';
  const pending = status === 'pending';
  const paymentReceivedAfterExpiry = isPaymentReceivedAfterExpiry(status, paymentPaidAt);
  const className = delivered
    ? 'bg-emerald-50 text-emerald-700'
    : pending
      ? 'bg-amber-50 text-amber-700'
      : paymentReceivedAfterExpiry
        ? 'bg-red-50 text-red-700'
        : 'bg-slate-100 text-slate-600';

  return <span className={`inline-flex rounded-md px-2 py-1 text-xs font-medium ${className}`}>{statusText(status, paymentPaidAt)}</span>;
}

function formatDate(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString('zh-CN', { hour12: false });
}

function StatusPanel({ products }: { products: Product[] }) {
  const totalStock = useMemo(() => products.reduce((sum, product) => sum + product.stock, 0), [products]);
  const totalSold = useMemo(() => products.reduce((sum, product) => sum + product.sold_count, 0), [products]);

  return (
    <div className="rounded-md border border-slate-200 bg-white p-5 shadow-panel">
      <h2 className="text-base font-semibold">当前状态</h2>
      <dl className="mt-4 space-y-3 text-sm">
        <div className="flex justify-between gap-3">
          <dt className="text-slate-500">商品种类</dt>
          <dd className="font-medium">{products.length}</dd>
        </div>
        <div className="flex justify-between gap-3">
          <dt className="text-slate-500">可售库存</dt>
          <dd className="font-medium">{totalStock}</dd>
        </div>
        <div className="flex justify-between gap-3">
          <dt className="text-slate-500">已售数量</dt>
          <dd className="font-medium">{totalSold}</dd>
        </div>
      </dl>
    </div>
  );
}

function isPaymentReceivedAfterExpiry(status: string, paymentPaidAt: string | null) {
  return status === 'expired' && paymentPaidAt !== null;
}

function statusText(status: string, paymentPaidAt: string | null = null) {
  if (isPaymentReceivedAfterExpiry(status, paymentPaidAt)) {
    return '异常订单';
  }
  const texts: Record<string, string> = {
    pending: '待支付',
    delivered: '已交付',
    expired: '已过期',
  };
  return texts[status] ?? status;
}
