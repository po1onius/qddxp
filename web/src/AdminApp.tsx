import { useEffect, useMemo, useRef, useState } from 'react';
import type { FormEvent, ReactNode } from 'react';
import {
  Boxes,
  ChevronLeft,
  ChevronRight,
  ClipboardList,
  Eye,
  EyeOff,
  FileText,
  Loader2,
  LogOut,
  Megaphone,
  PackagePlus,
  Pencil,
  Power,
  PowerOff,
  Plus,
  RefreshCcw,
  Save,
  ShieldCheck,
  Store,
  X,
} from 'lucide-react';
import { useToast } from './Toast';
import { StoreBrand } from './StoreBrand';
import {
  ADMIN_SESSION_EXPIRED_EVENT,
  ApiError,
  createAdminProduct,
  createProductInfo,
  getAdminAnnouncement,
  getAdminSession,
  listAdminApiCallLogs,
  listAdminOrders,
  listAdminProductInfo,
  listAdminProducts,
  listProducts,
  loginAdmin,
  logoutAdmin,
  updateAdminProductStatuses,
  updateAdminOrderRemark,
  updateAdminAnnouncement,
  updateProductInfo,
  updateProductInfoActive,
} from './api/client';
import type {
  AdminApiCallLog,
  AnnouncementSettings,
  AdminInventoryProduct,
  AdminOrder,
  AdminProductInfo,
  AdminProductStatus,
  CreateProductInfoInput,
  CreateAdminProductResult,
  Product,
  ProductInventoryStatus,
  StorefrontConfig,
} from './types';

const LEGACY_ADMIN_KEY_STORAGE = 'qddxp_admin_key';
const ADMIN_PAGE_SIZE = 20;
const PRODUCT_INFO_PAGE_SIZE = 8;
const MIN_PRODUCT_CONTENT_CHARS = 4;
const MAX_ORDER_REMARK_CHARS = 1000;
const MAX_ANNOUNCEMENT_CHARS = 10_000;

const inputClass =
  'mt-2 h-10 w-full rounded-md border border-slate-300 bg-white px-3 text-sm outline-none focus:border-slate-950';
const textareaClass =
  'mt-2 min-h-28 w-full rounded-md border border-slate-300 bg-white px-3 py-2 text-sm outline-none focus:border-slate-950';
const selectClass =
  'mt-2 h-10 w-full rounded-md border border-slate-300 bg-white px-3 text-sm outline-none focus:border-slate-950';
const currencyFormatter = new Intl.NumberFormat('zh-CN', {
  style: 'currency',
  currency: 'CNY',
});

type ProductOption = {
  id: string;
  image_base64: string | null;
  name: string;
  details: string;
  price_cents: number;
  sold_count: number;
  stock: number | null;
  active: boolean;
};

type ProductInfoFormState = {
  name: string;
  details: string;
  priceYuan: string;
  active: boolean;
  imageBase64: string;
  imageFile: File | null;
  imagePreviewUrl: string;
};

const emptyProductInfoForm: ProductInfoFormState = {
  name: '',
  details: '',
  priceYuan: '',
  active: true,
  imageBase64: '',
  imageFile: null,
  imagePreviewUrl: '',
};

type AdminTab = 'product_info' | 'inventory' | 'orders' | 'announcement' | 'logs';

type InventoryFilters = {
  productInfoId: string;
  status: '' | ProductInventoryStatus;
};

const emptyInventoryFilters: InventoryFilters = {
  productInfoId: '',
  status: '',
};

function totalPagesFor(total: number, pageSize: number) {
  return Math.max(1, Math.ceil(total / pageSize));
}

type AdminAuthState = 'checking' | 'authenticated' | 'unauthenticated';

export function AdminApp({ storefront }: { storefront: StorefrontConfig }) {
  const { showToast } = useToast();
  const [authState, setAuthState] = useState<AdminAuthState>('checking');
  const [loggingOut, setLoggingOut] = useState(false);

  useEffect(() => {
    let cancelled = false;

    // 旧版本曾把 ADMIN_KEY 明文保存在 localStorage。升级后立即清理遗留值，之后的
    // 认证凭据只在登录请求体中短暂存在，并由浏览器管理 HttpOnly 会话 Cookie。
    if (localStorage.getItem(LEGACY_ADMIN_KEY_STORAGE) !== null) {
      localStorage.removeItem(LEGACY_ADMIN_KEY_STORAGE);
      console.info('[管理员认证] 已清理旧版本在浏览器中保存的管理员密钥');
    }

    function handleSessionExpired() {
      console.warn('[管理员认证] 管理 API 返回未认证，会话可能已经过期');
      setAuthState('unauthenticated');
    }

    window.addEventListener(ADMIN_SESSION_EXPIRED_EVENT, handleSessionExpired);
    void getAdminSession()
      .then((status) => {
        if (cancelled) {
          return;
        }
        console.info('[管理员认证] 会话状态检查完成', { authenticated: status.authenticated });
        setAuthState(status.authenticated ? 'authenticated' : 'unauthenticated');
      })
      .catch((error) => {
        if (cancelled) {
          return;
        }
        console.error('[管理员认证] 会话状态检查失败', error);
        setAuthState('unauthenticated');
        showToast({
          message: error instanceof Error ? error.message : '管理员会话状态检查失败',
          type: 'error',
        });
      });

    return () => {
      cancelled = true;
      window.removeEventListener(ADMIN_SESSION_EXPIRED_EVENT, handleSessionExpired);
    };
  }, []);

  async function handleLogout() {
    setLoggingOut(true);
    try {
      await logoutAdmin();
      console.info('[管理员认证] 已退出并清除服务端会话');
      setAuthState('unauthenticated');
      showToast({ message: '已安全退出管理后台', type: 'success' });
    } catch (error) {
      console.error('[管理员认证] 退出登录失败', error);
      showToast({
        message: error instanceof Error ? error.message : '退出登录失败',
        type: 'error',
      });
    } finally {
      setLoggingOut(false);
    }
  }

  if (authState === 'checking') {
    return <AdminSessionLoadingPage storefront={storefront} />;
  }

  if (authState === 'unauthenticated') {
    return <AdminLoginPage onAuthenticated={() => setAuthState('authenticated')} storefront={storefront} />;
  }

  return (
    <AdminDashboard
      loggingOut={loggingOut}
      onLogout={() => void handleLogout()}
      storefront={storefront}
    />
  );
}

function AdminDashboard({
  loggingOut,
  onLogout,
  storefront,
}: {
  loggingOut: boolean;
  onLogout: () => void;
  storefront: StorefrontConfig;
}) {
  const { showToast } = useToast();
  const [products, setProducts] = useState<Product[]>([]);
  const [adminProductInfos, setAdminProductInfos] = useState<AdminProductInfo[]>([]);
  const [inventoryProducts, setInventoryProducts] = useState<AdminInventoryProduct[]>([]);
  const [inventoryFilters, setInventoryFilters] = useState<InventoryFilters>(emptyInventoryFilters);
  const [orders, setOrders] = useState<AdminOrder[]>([]);
  const [logs, setLogs] = useState<AdminApiCallLog[]>([]);
  const [inventoryPage, setInventoryPage] = useState(1);
  const [inventoryTotal, setInventoryTotal] = useState(0);
  const [ordersPage, setOrdersPage] = useState(1);
  const [ordersTotal, setOrdersTotal] = useState(0);
  const [logsPage, setLogsPage] = useState(1);
  const [logsTotal, setLogsTotal] = useState(0);
  const [activeTab, setActiveTab] = useState<AdminTab>('product_info');
  const [loadingProducts, setLoadingProducts] = useState(false);
  const [loadingProductInfos, setLoadingProductInfos] = useState(false);
  const [loadingInventory, setLoadingInventory] = useState(false);
  const [loadingOrders, setLoadingOrders] = useState(false);
  const [loadingLogs, setLoadingLogs] = useState(false);

  const productOptions = useMemo(() => mergeProductOptions(products, adminProductInfos), [products, adminProductInfos]);
  const inventoryProductOptions = useMemo(
    () => mergeInventoryProductOptions(productOptions, inventoryProducts),
    [inventoryProducts, productOptions],
  );
  useEffect(() => {
    void refreshProducts();
    void refreshProductInfo();
    void refreshOrders();
  }, []);

  async function refreshProducts() {
    setLoadingProducts(true);

    try {
      setProducts(await listProducts());
    } catch (err) {
      showToast({
        message: err instanceof Error ? err.message : '商品列表加载失败',
        type: 'error',
      });
    } finally {
      setLoadingProducts(false);
    }
  }

  async function refreshProductInfo() {
    setLoadingProductInfos(true);

    try {
      setAdminProductInfos(await listAdminProductInfo());
    } catch (err) {
      showToast({
        message: err instanceof Error ? err.message : '商品信息列表加载失败',
        type: 'error',
      });
    } finally {
      setLoadingProductInfos(false);
    }
  }

  async function refreshInventory(filters = inventoryFilters, page = inventoryPage) {
    await loadInventoryPage({ filters, page });
  }

  async function loadInventoryPage({
    errorMessage = '库存列表加载失败',
    filters = inventoryFilters,
    page,
  }: {
    errorMessage?: string;
    filters?: InventoryFilters;
    page: number;
  }) {
    setLoadingInventory(true);

    try {
      const response = await listAdminProducts({
        ...inventoryFilterParams(filters),
        page,
        page_size: ADMIN_PAGE_SIZE,
      });
      setInventoryProducts(response.items);
      setInventoryPage(response.page);
      setInventoryTotal(response.total);
    } catch (err) {
      showToast({
        message: err instanceof Error ? err.message : errorMessage,
        type: 'error',
      });
    } finally {
      setLoadingInventory(false);
    }
  }

  async function refreshOrders(page = ordersPage) {
    await loadOrdersPage({ page });
  }

  async function loadOrdersPage({
    errorMessage = '订单列表加载失败',
    page,
  }: {
    errorMessage?: string;
    page: number;
  }) {
    setLoadingOrders(true);

    try {
      const response = await listAdminOrders({ page, page_size: ADMIN_PAGE_SIZE });
      setOrders(response.items);
      setOrdersPage(response.page);
      setOrdersTotal(response.total);
    } catch (err) {
      showToast({
        message: err instanceof Error ? err.message : errorMessage,
        type: 'error',
      });
    } finally {
      setLoadingOrders(false);
    }
  }

  async function refreshLogs(page = logsPage) {
    await loadLogsPage({ page });
  }

  async function loadLogsPage({
    errorMessage = '日志列表加载失败',
    page,
  }: {
    errorMessage?: string;
    page: number;
  }) {
    setLoadingLogs(true);

    try {
      const response = await listAdminApiCallLogs({ page, page_size: ADMIN_PAGE_SIZE });
      setLogs(response.items);
      setLogsPage(response.page);
      setLogsTotal(response.total);
    } catch (err) {
      showToast({
        message: err instanceof Error ? err.message : errorMessage,
        type: 'error',
      });
    } finally {
      setLoadingLogs(false);
    }
  }

  function showError(message: string | null) {
    if (!message) {
      return;
    }
    showToast({ message, type: 'error' });
  }

  async function loadNextInventoryPage() {
    if (inventoryPage >= totalPagesFor(inventoryTotal, ADMIN_PAGE_SIZE)) {
      return;
    }

    await loadInventoryPage({
      errorMessage: '库存翻页失败',
      page: inventoryPage + 1,
    });
  }

  async function loadPreviousInventoryPage() {
    if (inventoryPage <= 1) {
      return;
    }

    await loadInventoryPage({
      errorMessage: '库存翻页失败',
      page: inventoryPage - 1,
    });
  }

  async function loadNextOrdersPage() {
    if (ordersPage >= totalPagesFor(ordersTotal, ADMIN_PAGE_SIZE)) {
      return;
    }

    await loadOrdersPage({
      errorMessage: '订单翻页失败',
      page: ordersPage + 1,
    });
  }

  async function loadPreviousOrdersPage() {
    if (ordersPage <= 1) {
      return;
    }

    await loadOrdersPage({
      errorMessage: '订单翻页失败',
      page: ordersPage - 1,
    });
  }

  async function loadNextLogsPage() {
    if (logsPage >= totalPagesFor(logsTotal, ADMIN_PAGE_SIZE)) {
      return;
    }

    await loadLogsPage({
      errorMessage: '日志翻页失败',
      page: logsPage + 1,
    });
  }

  async function loadPreviousLogsPage() {
    if (logsPage <= 1) {
      return;
    }

    await loadLogsPage({
      errorMessage: '日志翻页失败',
      page: logsPage - 1,
    });
  }

  function upsertProductInfo(info: AdminProductInfo) {
    setAdminProductInfos((current) => {
      const next = current.filter((item) => item.id !== info.id);
      return [info, ...next];
    });
  }

  function changeInventoryFilters(filters: InventoryFilters) {
    setInventoryFilters(filters);
    void refreshInventory(filters, 1);
  }

  function changeTab(tab: AdminTab) {
    setActiveTab(tab);
    if (tab === 'inventory') {
      void refreshInventory();
    }
    if (tab === 'orders') {
      void refreshOrders();
    }
    if (tab === 'logs') {
      void refreshLogs();
    }
  }

  return (
    <div className="min-h-screen bg-zinc-50 text-slate-950">
      <header className="border-b border-slate-200 bg-white">
        <div className="mx-auto flex max-w-7xl flex-col gap-4 px-4 py-4 lg:flex-row lg:items-center lg:justify-between lg:px-8">
          <StoreBrand storefront={storefront} />
          <div className="flex flex-col gap-3 sm:flex-row sm:items-end lg:justify-end">
            <button
              className="inline-flex h-10 items-center justify-center gap-2 rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500 disabled:cursor-wait disabled:opacity-60"
              disabled={loggingOut}
              onClick={onLogout}
              type="button"
            >
              {loggingOut ? <Loader2 className="animate-spin" size={18} /> : <LogOut size={18} />}
              退出登录
            </button>
            <button
              className="inline-flex h-10 items-center justify-center gap-2 rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500"
              onClick={() => (window.location.href = '/')}
              type="button"
            >
              <Store size={18} />
              商城
            </button>
          </div>
        </div>
      </header>

      <main className="mx-auto max-w-7xl px-4 py-6 lg:px-8">
        <AdminNav activeTab={activeTab} onChange={changeTab} />

        {activeTab === 'product_info' && (
          <ProductInfoCatalogPanel
            loading={loadingProducts || loadingProductInfos}
            onCreated={(info) => {
              upsertProductInfo(info);
              showToast({ message: `已创建商品信息：${info.name}`, type: 'success' });
              void refreshProducts();
            }}
            onError={showError}
            onUpdated={(info) => {
              upsertProductInfo(info);
              showToast({ message: `已更新商品信息：${info.name}`, type: 'success' });
              void refreshProducts();
            }}
            onStatusChanged={(info) => {
              upsertProductInfo(info);
              showToast({ message: `已${info.active ? '上架' : '下架'}商品信息：${info.name}`, type: 'success' });
              void refreshProducts();
            }}
            productOptions={productOptions}
          />
        )}

        {activeTab === 'inventory' && (
          <div className="mt-6">
            <InventoryProductsPanel
              filters={inventoryFilters}
              loading={loadingInventory}
              onFiltersChange={changeInventoryFilters}
              onInventoryCreated={(result) => {
                const stockedText = result.stocked > 0 ? `新增 ${result.stocked} 条可售库存` : '';
                const duplicateText = result.duplicates > 0 ? `，忽略 ${result.duplicates} 条重复卡密` : '';
                showToast({
                  message: `${stockedText || '没有新增库存'}${duplicateText}`,
                  type: 'success',
                });
                void refreshProducts();
                void refreshInventory(inventoryFilters, 1);
              }}
              onInventoryStatusChanged={(updated, ignored, status) => {
                const action = status === 'available' ? '上架' : '下架';
                const ignoredText = ignored > 0 ? `，已忽略 ${ignored} 条不可转换库存` : '';
                showToast({
                  message:
                    updated > 0
                      ? `已${action} ${updated} 条库存商品${ignoredText}`
                      : `没有可${action}的库存商品${ignoredText}`,
                  type: 'success',
                });
                void refreshProducts();
                void refreshInventory(inventoryFilters, inventoryPage);
              }}
              onError={showError}
              onNextPage={() => void loadNextInventoryPage()}
              onPreviousPage={() => void loadPreviousInventoryPage()}
              onRefresh={() => void refreshInventory()}
              page={inventoryPage}
              pageSize={ADMIN_PAGE_SIZE}
              productOptions={inventoryProductOptions}
              products={inventoryProducts}
              total={inventoryTotal}
            />
          </div>
        )}

        {activeTab === 'orders' && (
          <div className="mt-6">
            <OrdersPanel
              loading={loadingOrders}
              onNextPage={() => void loadNextOrdersPage()}
              onPreviousPage={() => void loadPreviousOrdersPage()}
              onRefresh={() => void refreshOrders()}
              onRemarkUpdated={(orderId, remark) => {
                // 使用接口返回的规范化文本就地更新当前页，避免保存后整页刷新造成滚动位置丢失。
                setOrders((current) =>
                  current.map((order) => (order.id === orderId ? { ...order, remark } : order)),
                );
              }}
              orders={orders}
              page={ordersPage}
              pageSize={ADMIN_PAGE_SIZE}
              total={ordersTotal}
            />
          </div>
        )}

        {activeTab === 'logs' && (
          <div className="mt-6">
            <ApiCallLogsPanel
              loading={loadingLogs}
              logs={logs}
              onNextPage={() => void loadNextLogsPage()}
              onPreviousPage={() => void loadPreviousLogsPage()}
              onRefresh={() => void refreshLogs()}
              page={logsPage}
              pageSize={ADMIN_PAGE_SIZE}
              total={logsTotal}
            />
          </div>
        )}

        {activeTab === 'announcement' && (
          <div className="mt-6">
            <AnnouncementSettingsPanel />
          </div>
        )}
      </main>
    </div>
  );
}

function AdminNav({ activeTab, onChange }: { activeTab: AdminTab; onChange: (tab: AdminTab) => void }) {
  const tabs: Array<{ icon: ReactNode; id: AdminTab; label: string }> = [
    { icon: <Boxes size={18} />, id: 'product_info', label: '商品信息' },
    { icon: <PackagePlus size={18} />, id: 'inventory', label: '库存' },
    { icon: <ClipboardList size={18} />, id: 'orders', label: '订单' },
    { icon: <Megaphone size={18} />, id: 'announcement', label: '公告设置' },
    { icon: <FileText size={18} />, id: 'logs', label: '日志' },
  ];

  return (
    <nav className="flex flex-wrap gap-2 border-b border-slate-200" aria-label="管理导航">
      {tabs.map((tab) => (
        <button
          className={`inline-flex h-11 items-center gap-2 border-b-2 px-3 text-sm font-medium ${
            activeTab === tab.id
              ? 'border-slate-950 text-slate-950'
              : 'border-transparent text-slate-500 hover:border-slate-300 hover:text-slate-800'
          }`}
          key={tab.id}
          onClick={() => onChange(tab.id)}
          type="button"
        >
          {tab.icon}
          {tab.label}
        </button>
      ))}
    </nav>
  );
}

function AnnouncementSettingsPanel() {
  const { showToast } = useToast();
  const [announcement, setAnnouncement] = useState('');
  const [savedSettings, setSavedSettings] = useState<AnnouncementSettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    console.info('[公告设置] 正在加载公告');
    void getAdminAnnouncement()
      .then((settings) => {
        if (cancelled) {
          return;
        }
        setAnnouncement(settings.announcement);
        setSavedSettings(settings);
        console.info('[公告设置] 公告加载完成', {
          announcementLength: Array.from(settings.announcement).length,
          updatedAt: settings.updated_at,
        });
      })
      .catch((error) => {
        if (cancelled) {
          return;
        }
        console.error('[公告设置] 公告加载失败', error);
        showToast({
          message: error instanceof Error ? error.message : '公告加载失败',
          type: 'error',
        });
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [showToast]);

  async function saveAnnouncement(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const announcementLength = Array.from(announcement.trim()).length;
    if (announcementLength > MAX_ANNOUNCEMENT_CHARS) {
      showToast({ message: `公告内容不能超过 ${MAX_ANNOUNCEMENT_CHARS} 个字符`, type: 'error' });
      return;
    }

    setSaving(true);
    console.info('[公告设置] 正在保存公告', {
      announcementLength,
      announcementEmpty: announcement.trim().length === 0,
    });
    try {
      const settings = await updateAdminAnnouncement({ announcement });
      setAnnouncement(settings.announcement);
      setSavedSettings(settings);
      console.info('[公告设置] 公告保存成功', {
        announcementLength: Array.from(settings.announcement).length,
        updatedAt: settings.updated_at,
      });
      showToast({ message: settings.announcement ? '公告已更新' : '公告已清空', type: 'success' });
    } catch (error) {
      console.error('[公告设置] 公告保存失败', error);
      showToast({
        message: error instanceof Error ? error.message : '公告保存失败',
        type: 'error',
      });
    } finally {
      setSaving(false);
    }
  }

  const announcementLength = Array.from(announcement).length;
  const unchanged = savedSettings !== null && announcement === savedSettings.announcement;

  return (
    <section className="rounded-md border border-slate-200 bg-white p-5 shadow-panel">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <h2 className="text-base font-semibold">商城公告</h2>
          <p className="mt-1 text-sm text-slate-500">保存后，顾客可通过商城右上角的“公告”按钮查看。</p>
        </div>
        {savedSettings && (
          <span className="text-xs text-slate-500">最后更新：{formatDate(savedSettings.updated_at)}</span>
        )}
      </div>
      <form className="mt-5" onSubmit={(event) => void saveAnnouncement(event)}>
        <label className="block">
          <span className="text-sm font-medium text-slate-700">公告内容</span>
          <textarea
            className="mt-2 min-h-72 w-full rounded-md border border-slate-300 bg-white px-3 py-2 text-sm leading-6 outline-none focus:border-slate-950"
            disabled={loading || saving}
            maxLength={MAX_ANNOUNCEMENT_CHARS}
            onChange={(event) => setAnnouncement(event.target.value)}
            placeholder="输入商城公告；留空时顾客端显示“暂无公告”"
            value={announcement}
          />
        </label>
        <div className="mt-2 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <span className={`text-xs ${announcementLength > MAX_ANNOUNCEMENT_CHARS ? 'text-red-600' : 'text-slate-500'}`}>
            {announcementLength} / {MAX_ANNOUNCEMENT_CHARS} 字符
          </span>
          <button
            className="inline-flex h-10 items-center justify-center gap-2 rounded-md bg-slate-950 px-4 text-sm font-medium text-white disabled:cursor-wait disabled:bg-slate-400"
            disabled={loading || saving || !savedSettings || unchanged || announcementLength > MAX_ANNOUNCEMENT_CHARS}
            type="submit"
          >
            {saving ? <Loader2 className="animate-spin" size={18} /> : <Save size={18} />}
            {saving ? '保存中' : '保存公告'}
          </button>
        </div>
      </form>
    </section>
  );
}

function AdminSessionLoadingPage({ storefront }: { storefront: StorefrontConfig }) {
  return (
    <main className="flex min-h-screen items-center justify-center bg-zinc-50 px-4 text-slate-950">
      <section className="w-full max-w-md rounded-md border border-slate-200 bg-white p-6 shadow-panel">
        <StoreBrand storefront={storefront} />
        <div className="mt-8 flex items-center gap-3 text-sm text-slate-600">
          <Loader2 className="animate-spin" size={20} />
          正在检查管理员会话…
        </div>
      </section>
    </main>
  );
}

function AdminLoginPage({
  onAuthenticated,
  storefront,
}: {
  onAuthenticated: () => void;
  storefront: StorefrontConfig;
}) {
  const [adminKey, setAdminKey] = useState('');
  const [showKey, setShowKey] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [errorMessage, setErrorMessage] = useState('');

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!adminKey) {
      setErrorMessage('请输入管理员密钥');
      return;
    }

    setSubmitting(true);
    setErrorMessage('');
    console.info('[管理员认证] 正在提交登录请求');
    try {
      const session = await loginAdmin(adminKey);
      if (!session.authenticated) {
        throw new Error('登录接口未建立管理员会话');
      }
      // 登录完成后尽快清掉组件内的密钥副本；后续请求只依赖 HttpOnly Cookie。
      setAdminKey('');
      console.info('[管理员认证] 登录成功，准备加载管理后台');
      onAuthenticated();
    } catch (error) {
      console.warn('[管理员认证] 登录失败', {
        status: error instanceof ApiError ? error.status : undefined,
        error,
      });
      setErrorMessage(
        error instanceof ApiError && error.status === 401
          ? '管理员密钥错误'
          : error instanceof Error
            ? error.message
            : '登录失败，请稍后重试',
      );
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <main className="flex min-h-screen items-center justify-center bg-zinc-50 px-4 py-10 text-slate-950">
      <section className="w-full max-w-md rounded-md border border-slate-200 bg-white p-6 shadow-panel">
        <StoreBrand storefront={storefront} />
        <div className="mt-8">
          <h1 className="text-xl font-semibold">登录管理后台</h1>
          <p className="mt-2 text-sm text-slate-500">请输入部署环境中配置的管理员密钥。</p>
        </div>

        <form className="mt-6" onSubmit={(event) => void submit(event)}>
          <label className="block" htmlFor="admin-key">
            <span className="text-sm font-medium text-slate-700">管理员密钥</span>
          </label>
          <div className="mt-2 flex">
          <input
            autoComplete="current-password"
            autoFocus
            className="h-11 min-w-0 flex-1 rounded-l-md border border-slate-300 px-3 text-sm outline-none focus:border-slate-950"
            disabled={submitting}
            id="admin-key"
            name="admin-key"
            onChange={(event) => setAdminKey(event.target.value)}
            type={showKey ? 'text' : 'password'}
            value={adminKey}
          />
          <button
            aria-label={showKey ? '隐藏管理员密钥' : '显示管理员密钥'}
            className="inline-flex h-11 w-11 items-center justify-center rounded-r-md border-y border-r border-slate-300 bg-white text-slate-600 hover:text-slate-950"
            onClick={() => setShowKey((current) => !current)}
            type="button"
          >
            {showKey ? <EyeOff size={18} /> : <Eye size={18} />}
          </button>
          </div>
          {errorMessage && (
            <p className="mt-3 rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700" role="alert">
              {errorMessage}
            </p>
          )}
          <button
            className="mt-5 inline-flex h-11 w-full items-center justify-center gap-2 rounded-md bg-slate-950 px-4 text-sm font-medium text-white disabled:cursor-wait disabled:bg-slate-400"
            disabled={submitting}
            type="submit"
          >
            {submitting ? <Loader2 className="animate-spin" size={18} /> : <ShieldCheck size={18} />}
            登录
          </button>
          <button
            className="mt-3 h-10 w-full rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500"
            onClick={() => (window.location.href = '/')}
            type="button"
          >
            返回商城
          </button>
        </form>
      </section>
    </main>
  );
}

function ProductInfoCatalogPanel({
  loading,
  onCreated,
  onError,
  onStatusChanged,
  onUpdated,
  productOptions,
}: {
  loading: boolean;
  onCreated: (info: AdminProductInfo) => void;
  onError: (message: string | null) => void;
  onStatusChanged: (info: AdminProductInfo) => void;
  onUpdated: (info: AdminProductInfo) => void;
  productOptions: ProductOption[];
}) {
  const [showCreateModal, setShowCreateModal] = useState(false);
  const [editingProduct, setEditingProduct] = useState<ProductOption | null>(null);
  const [showAllProductInfos, setShowAllProductInfos] = useState(false);
  const [updatingProductInfoId, setUpdatingProductInfoId] = useState<string | null>(null);
  const [page, setPage] = useState(1);
  // 默认只展示仍在售的商品信息；勾选“全部”后再把下架项纳入分页，避免隐藏项
  // 占用页码而造成某些页面显示数量不足。
  const filteredProductOptions = showAllProductInfos ? productOptions : productOptions.filter((product) => product.active);
  const totalPages = Math.max(1, Math.ceil(filteredProductOptions.length / PRODUCT_INFO_PAGE_SIZE));
  const visibleProductOptions = filteredProductOptions.slice(
    (page - 1) * PRODUCT_INFO_PAGE_SIZE,
    page * PRODUCT_INFO_PAGE_SIZE,
  );

  useEffect(() => {
    if (page > totalPages) {
      setPage(totalPages);
    }
  }, [page, totalPages]);

  async function toggleProductActive(product: ProductOption) {
    const active = !product.active;
    setUpdatingProductInfoId(product.id);
    onError(null);

    try {
      const info = await updateProductInfoActive(product.id, { active });
      onStatusChanged(info);
    } catch (err) {
      onError(err instanceof Error ? err.message : active ? '上架商品信息失败' : '下架商品信息失败');
    } finally {
      setUpdatingProductInfoId(null);
    }
  }

  return (
    <section className="mt-6">
      <div className="mb-4 flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <Boxes size={19} />
          <h2 className="text-base font-semibold">商品信息</h2>
        </div>
        <div className="flex flex-wrap items-center justify-end gap-2">
          <label className="inline-flex h-9 cursor-pointer items-center gap-2 rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500">
            <input
              checked={showAllProductInfos}
              className="h-4 w-4 accent-slate-950"
              onChange={(event) => {
                setShowAllProductInfos(event.target.checked);
                setPage(1);
              }}
              type="checkbox"
            />
            全部
          </label>
          {loading && <Loader2 className="animate-spin text-slate-500" size={18} />}
          {filteredProductOptions.length > PRODUCT_INFO_PAGE_SIZE && (
            <OffsetPaginationControls loading={loading} onPageChange={setPage} page={page} totalPages={totalPages} />
          )}
        </div>
      </div>

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
        <button
          className="flex min-h-[316px] flex-col items-center justify-center rounded-md border border-dashed border-slate-300 bg-white p-5 text-slate-500 shadow-panel hover:border-slate-500 hover:text-slate-950"
          onClick={() => setShowCreateModal(true)}
          type="button"
        >
          <span className="flex h-14 w-14 items-center justify-center rounded-md border border-slate-200 bg-slate-50">
            <Plus size={30} />
          </span>
          <span className="mt-3 text-sm font-medium">新增商品信息</span>
        </button>

        {visibleProductOptions.map((product) => (
          <ProductInfoCard
            key={product.id}
            onEdit={() => setEditingProduct(product)}
            onToggleActive={() => void toggleProductActive(product)}
            product={product}
            updating={updatingProductInfoId === product.id}
          />
        ))}
      </div>

      {productOptions.length > 0 && filteredProductOptions.length === 0 && !loading && (
        <p className="mt-4 rounded-md border border-slate-200 bg-white px-3 py-2 text-sm text-slate-500">
          暂无上架商品信息，勾选“全部”可查看下架商品。
        </p>
      )}

      {showCreateModal && (
        <ProductInfoModal
          onClose={() => setShowCreateModal(false)}
          onSaved={(info) => {
            setPage(1);
            onCreated(info);
          }}
          onError={onError}
        />
      )}

      {editingProduct && (
        <ProductInfoModal
          onClose={() => setEditingProduct(null)}
          onError={onError}
          onSaved={onUpdated}
          product={editingProduct}
        />
      )}
    </section>
  );
}

function ProductInfoCard({
  onEdit,
  onToggleActive,
  product,
  updating,
}: {
  onEdit: () => void;
  onToggleActive: () => void;
  product: ProductOption;
  updating: boolean;
}) {
  const imageSrc = product.image_base64 ? imageBase64Src(product.image_base64) : null;
  const actionText = product.active ? '下架' : '上架';

  return (
    <article
      className={`relative overflow-hidden rounded-md border bg-white text-left shadow-panel hover:border-slate-500 ${
        product.active ? 'border-slate-200' : 'border-slate-300'
      }`}
      data-active={product.active}
    >
      {/* 用覆盖卡片的独立按钮承接编辑操作，并把上下架按钮提升到更高层级。这样整张
          卡片都可点击，同时不会产生 button 嵌套，也保留完整的键盘焦点行为。 */}
      <button
        aria-label={`编辑商品信息 ${product.name}`}
        className="absolute inset-0 z-10 cursor-pointer rounded-md outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-slate-950"
        onClick={onEdit}
        type="button"
      />
      <div className={product.active ? undefined : 'grayscale opacity-70'}>
        <div className="flex h-48 items-center justify-center bg-slate-100">
          {imageSrc ? (
            <img alt={product.name} className="h-full w-full object-contain p-2" src={imageSrc} />
          ) : (
            <span className="px-4 text-center text-sm font-medium text-slate-500">{product.name}</span>
          )}
        </div>
        <div className="space-y-4 p-4">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <h3 className="truncate text-base font-semibold leading-6">{product.name}</h3>
              <p className="mt-1 break-all font-mono text-xs text-slate-400">{product.id}</p>
              <p className="mt-1 text-sm text-slate-500">
                库存 {product.stock ?? '-'} · 已售 {product.sold_count}
              </p>
            </div>
            <p className="shrink-0 text-base font-semibold text-emerald-700">{formatPrice(product.price_cents)}</p>
          </div>
          <div className="flex items-center justify-between gap-3">
            <div className="flex items-center gap-2">
              <StatusPill active={product.active} />
              <span className="inline-flex items-center gap-1 text-xs text-slate-500">
                <Pencil size={14} />
                点击编辑
              </span>
            </div>
            <button
              aria-label={`${actionText}商品信息 ${product.name}`}
              className="relative z-20 inline-flex h-9 items-center justify-center gap-2 rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500 disabled:cursor-wait disabled:opacity-60"
              disabled={updating}
              onClick={onToggleActive}
              type="button"
            >
              {updating ? <Loader2 className="animate-spin" size={17} /> : product.active ? <PowerOff size={17} /> : <Power size={17} />}
              {actionText}
            </button>
          </div>
        </div>
      </div>
      {!product.active && <div aria-hidden="true" className="pointer-events-none absolute inset-0 bg-slate-300/35" />}
    </article>
  );
}

function ProductInfoModal({
  onClose,
  onError,
  onSaved,
  product,
}: {
  onClose: () => void;
  onError: (message: string | null) => void;
  onSaved: (info: AdminProductInfo) => void;
  product?: ProductOption;
}) {
  const editing = product !== undefined;
  const [form, setForm] = useState<ProductInfoFormState>(() =>
    product
      ? {
          name: product.name,
          details: product.details,
          priceYuan: (product.price_cents / 100).toFixed(2),
          active: product.active,
          imageBase64: product.image_base64 ?? '',
          imageFile: null,
          imagePreviewUrl: '',
        }
      : emptyProductInfoForm,
  );
  const [submitting, setSubmitting] = useState(false);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const payload = await safeBuildProductInfoPayload(form, onError);
    if (!payload) {
      return;
    }

    setSubmitting(true);
    onError(null);

    try {
      // 上下架由商品卡片上的独立按钮负责；编辑仅提交展示字段，防止管理员在修改
      // 文案或价格时无意改变商品销售状态。
      const info = editing
        ? await updateProductInfo(product.id, {
            image_base64: payload.image_base64,
            name: payload.name,
            details: payload.details,
            price_cents: payload.price_cents,
          })
        : await createProductInfo(payload);
      console.info(editing ? '[商品信息] 编辑成功' : '[商品信息] 创建成功', {
        productInfoId: info.id,
        priceCents: info.price_cents,
      });
      onSaved(info);
      revokePreviewUrl(form);
      onClose();
    } catch (err) {
      console.error(editing ? '[商品信息] 编辑失败' : '[商品信息] 创建失败', {
        productInfoId: product?.id,
        error: err,
      });
      onError(err instanceof Error ? err.message : editing ? '更新商品信息失败' : '创建商品信息失败');
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="fixed inset-0 z-40 flex items-end justify-center bg-slate-950/45 px-4 py-4 sm:items-center">
      <section className="max-h-[calc(100vh-2rem)] w-full max-w-3xl overflow-hidden rounded-md bg-white shadow-panel">
        <div className="flex items-start justify-between gap-4 border-b border-slate-200 px-5 py-4">
          <div>
            <h2 className="text-lg font-semibold">{editing ? '编辑商品信息' : '新增商品信息'}</h2>
            <p className="mt-1 text-sm text-slate-500">
              {editing ? '修改后的信息会用于商城展示和后续新订单' : '填写商品基础信息和展示内容'}
            </p>
          </div>
          <button
            aria-label="关闭商品信息弹窗"
            className="rounded-md p-1 text-slate-500 hover:bg-slate-100 hover:text-slate-900"
            onClick={onClose}
            type="button"
          >
            <X size={20} />
          </button>
        </div>

        <form className="max-h-[calc(100vh-8rem)] overflow-auto p-5" noValidate onSubmit={submit}>
          <ProductInfoFields form={form} onChange={setForm} showActive={!editing} />
          <div className="mt-5 flex justify-end gap-3 border-t border-slate-200 pt-4">
            <button
              className="h-10 rounded-md border border-slate-300 bg-white px-4 text-sm font-medium text-slate-700 hover:border-slate-500"
              onClick={onClose}
              type="button"
            >
              取消
            </button>
            <button
              className="inline-flex h-10 items-center gap-2 rounded-md bg-slate-950 px-4 text-sm font-medium text-white disabled:cursor-wait disabled:bg-slate-400"
              disabled={submitting}
              type="submit"
            >
              {submitting ? <Loader2 className="animate-spin" size={18} /> : <Save size={18} />}
              {editing ? '保存' : '创建'}
            </button>
          </div>
        </form>
      </section>
    </div>
  );
}

function ProductInfoFields({
  form,
  onChange,
  showActive,
}: {
  form: ProductInfoFormState;
  onChange: (form: ProductInfoFormState) => void;
  showActive: boolean;
}) {
  const imageInputRef = useRef<HTMLInputElement>(null);
  const previewSrc = form.imagePreviewUrl || (form.imageBase64.trim() ? imageBase64Src(form.imageBase64.trim()) : null);

  useEffect(() => () => revokePreviewUrl(form), [form.imagePreviewUrl]);

  function changeImageFile(file: File | null) {
    revokePreviewUrl(form);
    onChange({
      ...form,
      imageFile: file,
      imagePreviewUrl: file ? URL.createObjectURL(file) : '',
    });
  }

  function clearImage() {
    revokePreviewUrl(form);
    if (imageInputRef.current) {
      imageInputRef.current.value = '';
    }
    onChange({
      ...form,
      imageBase64: '',
      imageFile: null,
      imagePreviewUrl: '',
    });
  }

  return (
    <>
      <label className="mt-4 block">
        <span className="text-sm font-medium text-slate-700">名称</span>
        <input className={inputClass} onChange={(event) => onChange({ ...form, name: event.target.value })} required value={form.name} />
      </label>
      <label className="mt-4 block">
        <span className="text-sm font-medium text-slate-700">详情</span>
        <textarea
          className={textareaClass}
          onChange={(event) => onChange({ ...form, details: event.target.value })}
          value={form.details ?? ''}
        />
      </label>
      <label className="mt-4 block">
        <span className="text-sm font-medium text-slate-700">价格</span>
        <input
          className={inputClass}
          min="0"
          onChange={(event) => onChange({ ...form, priceYuan: event.target.value })}
          placeholder="0.00"
          required
          step="0.01"
          type="number"
          value={form.priceYuan}
        />
      </label>
      {showActive && (
        <label className="mt-4 flex items-center gap-2 text-sm font-medium text-slate-700">
          <input
            checked={form.active}
            className="h-4 w-4 rounded border-slate-300"
            onChange={(event) => onChange({ ...form, active: event.target.checked })}
            type="checkbox"
          />
          上架
        </label>
      )}
      <div className="mt-4">
        <span className="text-sm font-medium text-slate-700">商品图</span>
        <div className="mt-2 flex flex-col gap-2 sm:flex-row sm:items-center">
          <input
            accept="image/*"
            className="hidden"
            onChange={(event) => changeImageFile(event.target.files?.[0] ?? null)}
            ref={imageInputRef}
            type="file"
          />
          <button
            className="inline-flex h-10 items-center justify-center rounded-md bg-slate-950 px-3 text-sm font-medium text-white hover:bg-slate-800"
            onClick={() => imageInputRef.current?.click()}
            type="button"
          >
            选择图片
          </button>
          <span className="min-w-0 text-sm text-slate-500">
            {form.imageFile?.name ?? (form.imageBase64 ? '已选择图片' : '未选择图片')}
          </span>
          {previewSrc && (
            <button
              className="h-10 shrink-0 rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500"
              onClick={clearImage}
              type="button"
            >
              清除图片
            </button>
          )}
        </div>
      </div>
      {previewSrc && (
        <div className="mt-3 flex h-64 items-center justify-center rounded-md border border-slate-200 bg-slate-100">
          <img alt="" className="h-full w-full object-contain p-2" src={previewSrc} />
        </div>
      )}
    </>
  );
}

function InventoryProductsPanel({
  filters,
  loading,
  onFiltersChange,
  onInventoryCreated,
  onInventoryStatusChanged,
  onError,
  onNextPage,
  onPreviousPage,
  onRefresh,
  page,
  pageSize,
  productOptions,
  products,
  total,
}: {
  filters: InventoryFilters;
  loading: boolean;
  onFiltersChange: (filters: InventoryFilters) => void;
  onInventoryCreated: (result: CreateAdminProductResult) => void;
  onInventoryStatusChanged: (updated: number, ignored: number, status: AdminProductStatus) => void;
  onError: (message: string | null) => void;
  onNextPage: () => void;
  onPreviousPage: () => void;
  onRefresh: () => void;
  page: number;
  pageSize: number;
  productOptions: ProductOption[];
  products: AdminInventoryProduct[];
  total: number;
}) {
  const [showCreateModal, setShowCreateModal] = useState(false);
  const [selectedProductIds, setSelectedProductIds] = useState<Set<string>>(() => new Set());
  const [updatingStatus, setUpdatingStatus] = useState<AdminProductStatus | null>(null);
  const allPageSelected = products.length > 0 && products.every((product) => selectedProductIds.has(product.id));
  const selectedCount = selectedProductIds.size;

  useEffect(() => {
    setSelectedProductIds(new Set());
  }, [products]);

  function togglePageSelection(checked: boolean) {
    setSelectedProductIds(checked ? new Set(products.map((product) => product.id)) : new Set());
  }

  function toggleProductSelection(productId: string, checked: boolean) {
    setSelectedProductIds((current) => {
      const next = new Set(current);
      if (checked) {
        next.add(productId);
      } else {
        next.delete(productId);
      }
      return next;
    });
  }

  async function updateSelectedStatus(status: AdminProductStatus) {
    const productIds = Array.from(selectedProductIds);
    if (productIds.length === 0) {
      onError('请先选择库存商品');
      return;
    }
    setUpdatingStatus(status);
    onError(null);

    try {
      const result = await updateAdminProductStatuses({
        product_ids: productIds,
        status,
      });
      setSelectedProductIds(new Set());
      onInventoryStatusChanged(result.updated, result.ignored, status);
    } catch (err) {
      onError(err instanceof Error ? err.message : status === 'available' ? '上架库存商品失败' : '下架库存商品失败');
    } finally {
      setUpdatingStatus(null);
    }
  }

  return (
    <section className="rounded-md border border-slate-200 bg-white p-5 shadow-panel">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-2">
          <PackagePlus size={19} />
          <h2 className="text-base font-semibold">库存列表</h2>
        </div>
        <div className="flex flex-col gap-2 sm:flex-row">
          <button
            className="inline-flex h-10 items-center justify-center gap-2 rounded-md bg-slate-950 px-3 text-sm font-medium text-white hover:bg-slate-800"
            onClick={() => setShowCreateModal(true)}
            type="button"
          >
            <Plus size={18} />
            补充库存
          </button>
          <button
            className="inline-flex h-10 items-center justify-center gap-2 rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500 disabled:cursor-not-allowed disabled:opacity-50"
            disabled={loading || Boolean(updatingStatus) || selectedCount === 0}
            onClick={() => void updateSelectedStatus('disabled')}
            type="button"
          >
            {updatingStatus === 'disabled' ? <Loader2 className="animate-spin" size={18} /> : <PowerOff size={18} />}
            下架
          </button>
          <button
            className="inline-flex h-10 items-center justify-center gap-2 rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500 disabled:cursor-not-allowed disabled:opacity-50"
            disabled={loading || Boolean(updatingStatus) || selectedCount === 0}
            onClick={() => void updateSelectedStatus('available')}
            type="button"
          >
            {updatingStatus === 'available' ? <Loader2 className="animate-spin" size={18} /> : <Power size={18} />}
            上架
          </button>
          <button
            className="inline-flex h-10 items-center justify-center gap-2 rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500 disabled:cursor-wait disabled:opacity-60"
            disabled={loading}
            onClick={onRefresh}
            type="button"
          >
            {loading ? <Loader2 className="animate-spin" size={18} /> : <RefreshCcw size={18} />}
            刷新
          </button>
        </div>
      </div>

      <div className="mt-4 grid gap-3 md:grid-cols-[minmax(0,1fr)_220px_auto] md:items-end">
        <label className="block">
          <span className="text-sm font-medium text-slate-700">商品</span>
          <select
            className={selectClass}
            onChange={(event) => onFiltersChange({ ...filters, productInfoId: event.target.value })}
            value={filters.productInfoId}
          >
            <option value="">全部商品</option>
            {productOptions.map((product) => (
              <option key={product.id} value={product.id}>
                {product.name}
              </option>
            ))}
          </select>
        </label>
        <label className="block">
          <span className="text-sm font-medium text-slate-700">状态</span>
          <select
            className={selectClass}
            onChange={(event) =>
              onFiltersChange({
                ...filters,
                status: event.target.value as InventoryFilters['status'],
              })
            }
            value={filters.status}
          >
            <option value="">全部状态</option>
            <option value="available">可售</option>
            <option value="reserved">已预留</option>
            <option value="delivered">已发货</option>
            <option value="disabled">禁用</option>
          </select>
        </label>
        <button
          className="inline-flex h-10 items-center justify-center rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500 disabled:cursor-not-allowed disabled:opacity-50"
          disabled={!filters.productInfoId && !filters.status}
          onClick={() => onFiltersChange(emptyInventoryFilters)}
          type="button"
        >
          重置
        </button>
      </div>

      <div className="mt-4 overflow-x-auto">
        <table className="min-w-[1060px] text-left text-sm">
          <thead className="border-b border-slate-200 text-xs uppercase text-slate-500">
            <tr>
              <th className="w-[92px] py-2 pr-3 font-medium">
                <label className="inline-flex items-center gap-2">
                  <input
                    checked={allPageSelected}
                    className="h-4 w-4 rounded border-slate-300"
                    disabled={products.length === 0 || loading || Boolean(updatingStatus)}
                    onChange={(event) => togglePageSelection(event.target.checked)}
                    type="checkbox"
                  />
                  全选
                </label>
              </th>
              <th className="py-2 pr-3 font-medium">库存 ID</th>
              <th className="px-3 py-2 font-medium">商品</th>
              <th className="px-3 py-2 font-medium">价格</th>
              <th className="px-3 py-2 font-medium">状态</th>
              <th className="px-3 py-2 font-medium">入库时间</th>
              <th className="px-3 py-2 font-medium">发货内容</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-slate-100">
            {products.map((product) => (
              <tr key={product.id} className="align-top">
                <td className="py-2 pr-3">
                  <input
                    aria-label={`选择库存 ${product.id}`}
                    checked={selectedProductIds.has(product.id)}
                    className="h-4 w-4 rounded border-slate-300"
                    disabled={loading || Boolean(updatingStatus)}
                    onChange={(event) => toggleProductSelection(product.id, event.target.checked)}
                    type="checkbox"
                  />
                </td>
                <td className="max-w-[160px] py-2 pr-3 font-mono text-xs text-slate-600">{product.id}</td>
                <td className="px-3 py-2">
                  <p className="font-medium">{product.product_name}</p>
                  <p className="mt-1 text-xs text-slate-500">{product.product_info_active ? '上架' : '下架'}</p>
                </td>
                <td className="px-3 py-2 font-medium text-emerald-700">{formatPrice(product.price_cents)}</td>
                <td className="px-3 py-2">
                  <ProductStatusBadge status={product.status} />
                </td>
                <td className="px-3 py-2 text-slate-600">{formatDate(product.created_at)}</td>
                <td className="max-w-[300px] px-3 py-2">
                  <pre className="max-h-20 overflow-auto whitespace-pre-wrap break-words rounded-md bg-slate-50 p-2 text-xs text-slate-700">
                    {product.content}
                  </pre>
                </td>
              </tr>
            ))}
            {products.length === 0 && (
              <tr>
                <td className="py-6 text-sm text-slate-500" colSpan={7}>
                  暂无库存
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      <TablePaginationControls
        currentCount={products.length}
        loading={loading}
        onNextPage={onNextPage}
        onPreviousPage={onPreviousPage}
        page={page}
        pageSize={pageSize}
        total={total}
      />
      {showCreateModal && (
        <StockCreateModal
          initialProductInfoId={filters.productInfoId}
          onClose={() => setShowCreateModal(false)}
          onCreated={(result) => {
            setShowCreateModal(false);
            onInventoryCreated(result);
          }}
          onError={onError}
          productOptions={productOptions}
        />
      )}
    </section>
  );
}

function TablePaginationControls({
  currentCount,
  loading,
  onNextPage,
  onPreviousPage,
  page,
  pageSize,
  total,
}: {
  currentCount: number;
  loading: boolean;
  onNextPage: () => void;
  onPreviousPage: () => void;
  page: number;
  pageSize: number;
  total: number;
}) {
  const totalPages = totalPagesFor(total, pageSize);

  return (
    <div className="mt-4 flex flex-col gap-3 border-t border-slate-200 pt-4 sm:flex-row sm:items-center sm:justify-between">
      <p className="text-sm text-slate-500">
        第 {page} / {totalPages} 页 · 当前 {currentCount} 条 · 共 {total} 条
      </p>
      <div className="flex flex-wrap gap-2">
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
    </div>
  );
}

function OffsetPaginationControls({
  loading,
  onPageChange,
  page,
  totalPages,
}: {
  loading: boolean;
  onPageChange: (page: number) => void;
  page: number;
  totalPages: number;
}) {
  return (
    <div className="flex flex-wrap gap-2">
      <button
        className="inline-flex h-9 items-center justify-center rounded-md border border-slate-300 bg-white px-2 text-sm font-medium text-slate-700 hover:border-slate-500 disabled:cursor-not-allowed disabled:opacity-50"
        disabled={loading || page <= 1}
        onClick={() => onPageChange(page - 1)}
        type="button"
      >
        <ChevronLeft size={18} />
      </button>
      <span className="inline-flex h-9 items-center rounded-md border border-slate-200 bg-white px-3 text-sm text-slate-600">
        {page} / {totalPages}
      </span>
      <button
        className="inline-flex h-9 items-center justify-center rounded-md border border-slate-300 bg-white px-2 text-sm font-medium text-slate-700 hover:border-slate-500 disabled:cursor-not-allowed disabled:opacity-50"
        disabled={loading || page >= totalPages}
        onClick={() => onPageChange(page + 1)}
        type="button"
      >
        <ChevronRight size={18} />
      </button>
    </div>
  );
}

function StockCreateModal({
  initialProductInfoId,
  onClose,
  onCreated,
  onError,
  productOptions,
}: {
  initialProductInfoId: string;
  onClose: () => void;
  onCreated: (result: CreateAdminProductResult) => void;
  onError: (message: string | null) => void;
  productOptions: ProductOption[];
}) {
  const initialProductExists = productOptions.some((product) => product.id === initialProductInfoId);
  const [productInfoId, setProductInfoId] = useState(initialProductExists ? initialProductInfoId : productOptions[0]?.id ?? '');
  const [content, setContent] = useState('');
  const [separator, setSeparator] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const parsedContentCount = splitInventoryContents(content, separator).length;

  useEffect(() => {
    if (!productInfoId && productOptions[0]) {
      setProductInfoId(productOptions[0].id);
    }
  }, [productInfoId, productOptions]);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const contents = splitInventoryContents(content, separator);

    if (!productInfoId) {
      onError('请选择商品信息');
      return;
    }
    if (contents.length === 0) {
      onError('库存内容不能为空');
      return;
    }
    // Array.from 按 Unicode 码点计数，与后端 Rust chars() 和 PostgreSQL char_length
    // 保持一致；String.length 统计 UTF-16 码元，会把部分 emoji 错算成两位。
    const firstInvalidContentIndex = contents.findIndex(
      (item) => Array.from(item).length < MIN_PRODUCT_CONTENT_CHARS,
    );
    if (firstInvalidContentIndex !== -1) {
      console.warn('[库存补货] 发货内容长度校验失败', {
        itemCount: contents.length,
        firstInvalidItemNumber: firstInvalidContentIndex + 1,
        minimumContentChars: MIN_PRODUCT_CONTENT_CHARS,
      });
      onError(`第 ${firstInvalidContentIndex + 1} 条发货内容少于 ${MIN_PRODUCT_CONTENT_CHARS} 位`);
      return;
    }

    setSubmitting(true);
    onError(null);
    console.info('[库存补货] 准备提交批量库存', {
      productInfoId,
      rawContentLength: content.length,
      separatorLength: separator.length,
      usesDefaultLineSeparator: separator.length === 0,
      itemCount: contents.length,
    });

    try {
      const createdProducts = await createAdminProduct({
        product_info_id: productInfoId,
        contents,
      });
      console.info('[库存补货] 批量库存提交成功', {
        productInfoId,
        submitted: createdProducts.submitted,
        stocked: createdProducts.stocked,
        duplicates: createdProducts.duplicates,
      });
      onCreated(createdProducts);
    } catch (err) {
      console.error('[库存补货] 批量库存提交失败', {
        productInfoId,
        itemCount: contents.length,
        error: err,
      });
      onError(err instanceof Error ? err.message : '添加库存商品失败');
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="fixed inset-0 z-40 flex items-end justify-center bg-slate-950/45 px-4 py-4 sm:items-center">
      <section className="w-full max-w-xl overflow-hidden rounded-md bg-white shadow-panel">
        <div className="flex items-start justify-between gap-4 border-b border-slate-200 px-5 py-4">
          <div>
            <h2 className="text-lg font-semibold">补充库存</h2>
            <p className="mt-1 text-sm text-slate-500">添加可售库存商品</p>
          </div>
          <button
            aria-label="关闭补充库存弹窗"
            className="rounded-md p-1 text-slate-500 hover:bg-slate-100 hover:text-slate-900"
            onClick={onClose}
            type="button"
          >
            <X size={20} />
          </button>
        </div>

        <form className="p-5" onSubmit={submit}>
          <label className="block">
            <span className="text-sm font-medium text-slate-700">商品信息</span>
            <select className={selectClass} onChange={(event) => setProductInfoId(event.target.value)} value={productInfoId}>
              {productOptions.map((product) => (
                <option key={product.id} value={product.id}>
                  {product.name}
                </option>
              ))}
            </select>
          </label>
          <label className="mt-4 block">
            <span className="text-sm font-medium text-slate-700">发货内容</span>
            <textarea
              aria-describedby="stock-content-help"
              className={textareaClass}
              onChange={(event) => setContent(event.target.value)}
              placeholder={`输入多条发货内容，每条至少 ${MIN_PRODUCT_CONTENT_CHARS} 位`}
              value={content}
            />
          </label>
          <p className="mt-2 text-xs text-slate-500" id="stock-content-help">
            每条发货内容清理首尾空白后不得少于 {MIN_PRODUCT_CONTENT_CHARS} 位。
          </p>
          <label className="mt-4 block">
            <span className="text-sm font-medium text-slate-700">自定义分隔符</span>
            <input
              aria-describedby="stock-separator-help"
              className={inputClass}
              onChange={(event) => setSeparator(event.target.value)}
              placeholder="留空时默认按换行分隔，例如：---"
              value={separator}
            />
          </label>
          <p className="mt-2 text-xs text-slate-500" id="stock-separator-help">
            分隔符按输入文本精确匹配，不是正则表达式；留空时一行一条。当前识别 {parsedContentCount} 条库存。
          </p>
          <div className="mt-5 flex justify-end gap-3 border-t border-slate-200 pt-4">
            <button
              className="h-10 rounded-md border border-slate-300 bg-white px-4 text-sm font-medium text-slate-700 hover:border-slate-500"
              onClick={onClose}
              type="button"
            >
              取消
            </button>
            <button
              className="inline-flex h-10 items-center gap-2 rounded-md bg-slate-950 px-4 text-sm font-medium text-white disabled:cursor-wait disabled:bg-slate-400"
              disabled={submitting || productOptions.length === 0}
              type="submit"
            >
              {submitting ? <Loader2 className="animate-spin" size={18} /> : <Save size={18} />}
              添加
            </button>
          </div>
        </form>
      </section>
    </div>
  );
}

function OrdersPanel({
  loading,
  onNextPage,
  onPreviousPage,
  onRefresh,
  onRemarkUpdated,
  orders,
  page,
  pageSize,
  total,
}: {
  loading: boolean;
  onNextPage: () => void;
  onPreviousPage: () => void;
  onRefresh: () => void;
  onRemarkUpdated: (orderId: string, remark: string) => void;
  orders: AdminOrder[];
  page: number;
  pageSize: number;
  total: number;
}) {
  return (
    <section className="rounded-md border border-slate-200 bg-white p-5 shadow-panel">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-2">
          <ClipboardList size={19} />
          <h2 className="text-base font-semibold">订单列表</h2>
        </div>
        <button
          className="inline-flex h-10 items-center gap-2 rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500 disabled:cursor-wait disabled:opacity-60"
          disabled={loading}
          onClick={onRefresh}
          type="button"
        >
          {loading ? <Loader2 className="animate-spin" size={18} /> : <RefreshCcw size={18} />}
          刷新
        </button>
      </div>

      <div className="mt-4 overflow-x-auto">
        <table className="min-w-[1460px] text-left text-sm">
          <thead className="border-b border-slate-200 text-xs uppercase text-slate-500">
            <tr>
              <th className="py-2 pr-3 font-medium">订单号</th>
              <th className="px-3 py-2 font-medium">商品</th>
              <th className="px-3 py-2 font-medium">联系方式</th>
              <th className="px-3 py-2 font-medium">状态</th>
              <th className="px-3 py-2 font-medium">支付渠道</th>
              <th className="px-3 py-2 font-medium">下单时间</th>
              <th className="px-3 py-2 font-medium">发货内容</th>
              <th className="px-3 py-2 font-medium">备注</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-slate-100">
            {orders.map((order) => (
              <tr key={order.id} className="align-top">
                <td className="max-w-[160px] py-2 pr-3 font-mono text-xs text-slate-600">{order.id}</td>
                <td className="px-3 py-2">
                  <p className="font-medium">{order.product_name}</p>
                </td>
                <td className="max-w-[180px] px-3 py-2 text-slate-600">{order.contact}</td>
                <td className="px-3 py-2">
                  <OrderStatusBadge paymentPaidAt={order.payment_paid_at} status={order.status} />
                </td>
                <td className="max-w-[220px] px-3 py-2 text-xs text-slate-600">
                  <p className="font-medium text-slate-800">{paymentMethodText(order.payment_provider, order.payment_channel)}</p>
                  <p className="mt-1">支付状态：{order.payment_state}</p>
                  <p className="mt-1 font-mono break-all">{order.provider_transaction_id ?? order.merchant_trade_no}</p>
                </td>
                <td className="px-3 py-2 text-slate-600">{formatDate(order.created_at)}</td>
                <td className="max-w-[260px] px-3 py-2">
                  <pre className="max-h-20 overflow-auto whitespace-pre-wrap break-words rounded-md bg-slate-50 p-2 text-xs text-slate-700">
                    {isPaymentReceivedAfterExpiry(order.status, order.payment_paid_at)
                      ? '异常订单，联系管理员处理'
                      : order.product_content ?? '未发货'}
                  </pre>
                </td>
                <td className="w-[300px] px-3 py-2">
                  <OrderRemarkEditor
                    onUpdated={(remark) => onRemarkUpdated(order.id, remark)}
                    order={order}
                  />
                </td>
              </tr>
            ))}
            {orders.length === 0 && (
              <tr>
                <td className="py-6 text-sm text-slate-500" colSpan={8}>
                  暂无订单
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      <TablePaginationControls
        currentCount={orders.length}
        loading={loading}
        onNextPage={onNextPage}
        onPreviousPage={onPreviousPage}
        page={page}
        pageSize={pageSize}
        total={total}
      />
    </section>
  );
}

function OrderRemarkEditor({
  onUpdated,
  order,
}: {
  onUpdated: (remark: string) => void;
  order: AdminOrder;
}) {
  const { showToast } = useToast();
  const [draft, setDraft] = useState(order.remark);
  const [saving, setSaving] = useState(false);

  // 翻页、刷新或其他保存结果改变当前订单时，让编辑器同步数据库中的最新备注。
  useEffect(() => {
    setDraft(order.remark);
  }, [order.id, order.remark]);

  const normalizedDraft = draft.trim();
  // Array.from 按 Unicode 码点计数，与后端 Rust chars 和 PostgreSQL char_length 的语义一致。
  const remarkChars = Array.from(normalizedDraft).length;
  const tooLong = remarkChars > MAX_ORDER_REMARK_CHARS;
  const changed = normalizedDraft !== order.remark;

  async function handleSave() {
    if (!changed || tooLong || saving) {
      return;
    }

    setSaving(true);
    try {
      const result = await updateAdminOrderRemark(order.id, { remark: draft });
      setDraft(result.remark);
      onUpdated(result.remark);
      showToast({ message: result.remark ? '订单备注已保存' : '订单备注已清空', type: 'success' });
    } catch (err) {
      showToast({
        message: err instanceof Error ? err.message : '订单备注保存失败',
        type: 'error',
      });
    } finally {
      setSaving(false);
    }
  }

  return (
    <div>
      <textarea
        aria-label={`订单 ${order.id} 的备注`}
        className="min-h-20 w-full resize-y rounded-md border border-slate-300 bg-white px-2 py-2 text-sm outline-none focus:border-slate-950"
        disabled={saving}
        onChange={(event) => setDraft(event.target.value)}
        placeholder="暂无备注"
        value={draft}
      />
      <div className="mt-1 flex items-center justify-between gap-2">
        <span className={`text-xs ${tooLong ? 'text-red-600' : 'text-slate-400'}`}>
          {remarkChars}/{MAX_ORDER_REMARK_CHARS}
        </span>
        <div className="flex gap-2">
          {changed && (
            <button
              className="h-8 rounded-md border border-slate-300 bg-white px-2 text-xs font-medium text-slate-600 hover:border-slate-500 disabled:opacity-50"
              disabled={saving}
              onClick={() => setDraft(order.remark)}
              type="button"
            >
              撤销
            </button>
          )}
          <button
            className="inline-flex h-8 items-center gap-1 rounded-md bg-slate-950 px-2 text-xs font-medium text-white disabled:cursor-not-allowed disabled:bg-slate-300"
            disabled={!changed || tooLong || saving}
            onClick={() => void handleSave()}
            type="button"
          >
            {saving ? <Loader2 className="animate-spin" size={14} /> : <Save size={14} />}
            保存
          </button>
        </div>
      </div>
    </div>
  );
}

function ApiCallLogsPanel({
  loading,
  logs,
  onNextPage,
  onPreviousPage,
  onRefresh,
  page,
  pageSize,
  total,
}: {
  loading: boolean;
  logs: AdminApiCallLog[];
  onNextPage: () => void;
  onPreviousPage: () => void;
  onRefresh: () => void;
  page: number;
  pageSize: number;
  total: number;
}) {
  return (
    <section className="rounded-md border border-slate-200 bg-white p-5 shadow-panel">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-2">
          <FileText size={19} />
          <h2 className="text-base font-semibold">API 调用日志</h2>
        </div>
        <button
          className="inline-flex h-10 items-center gap-2 rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500 disabled:cursor-wait disabled:opacity-60"
          disabled={loading}
          onClick={onRefresh}
          type="button"
        >
          {loading ? <Loader2 className="animate-spin" size={18} /> : <RefreshCcw size={18} />}
          刷新
        </button>
      </div>

      <div className="mt-4 overflow-x-auto">
        <table className="min-w-[1120px] text-left text-sm">
          <thead className="border-b border-slate-200 text-xs uppercase text-slate-500">
            <tr>
              <th className="py-2 pr-3 font-medium">时间</th>
              <th className="px-3 py-2 font-medium">接口</th>
              <th className="px-3 py-2 font-medium">方法</th>
              <th className="px-3 py-2 font-medium">请求参数</th>
              <th className="px-3 py-2 font-medium">响应</th>
              <th className="px-3 py-2 font-medium">结果</th>
              <th className="px-3 py-2 font-medium">错误</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-slate-100">
            {logs.map((log) => (
              <tr key={log.id} className="align-top">
                <td className="w-[150px] py-2 pr-3 text-slate-600">{formatDate(log.created_at)}</td>
                <td className="w-[180px] px-3 py-2">
                  <p className="font-medium">{log.api_name}</p>
                  <p className="mt-1 break-all font-mono text-xs text-slate-500">{log.path}</p>
                </td>
                <td className="w-[90px] px-3 py-2">
                  <span className="inline-flex rounded-md bg-slate-100 px-2 py-1 font-mono text-xs font-medium text-slate-700">
                    {log.http_method}
                  </span>
                </td>
                <td className="max-w-[300px] px-3 py-2">
                  <pre className="max-h-28 overflow-auto whitespace-pre-wrap break-words rounded-md bg-slate-50 p-2 text-xs text-slate-700">
                    {formatJson(log.request_params)}
                  </pre>
                </td>
                <td className="max-w-[180px] px-3 py-2">
                  <p className="font-mono text-xs text-slate-500">HTTP {log.response_status}</p>
                  <pre className="mt-1 max-h-20 overflow-auto whitespace-pre-wrap break-words rounded-md bg-slate-50 p-2 text-xs text-slate-700">
                    {log.response_body}
                  </pre>
                </td>
                <td className="px-3 py-2">
                  <LogStatusBadge success={log.success} />
                </td>
                <td className="max-w-[220px] px-3 py-2 text-slate-600">
                  {log.error_message ? (
                    <pre className="max-h-20 overflow-auto whitespace-pre-wrap break-words rounded-md bg-red-50 p-2 text-xs text-red-700">
                      {log.error_message}
                    </pre>
                  ) : (
                    <span className="text-slate-400">-</span>
                  )}
                </td>
              </tr>
            ))}
            {logs.length === 0 && (
              <tr>
                <td className="py-6 text-sm text-slate-500" colSpan={7}>
                  暂无日志
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      <TablePaginationControls
        currentCount={logs.length}
        loading={loading}
        onNextPage={onNextPage}
        onPreviousPage={onPreviousPage}
        page={page}
        pageSize={pageSize}
        total={total}
      />
    </section>
  );
}

function StatusPill({ active }: { active: boolean }) {
  return (
    <span
      className={`inline-flex rounded-md px-2 py-1 text-xs font-medium ${
        active ? 'bg-emerald-50 text-emerald-700' : 'bg-slate-100 text-slate-600'
      }`}
    >
      {active ? '上架' : '下架'}
    </span>
  );
}

function ProductStatusBadge({ status }: { status: string }) {
  const className =
    status === 'available'
      ? 'bg-emerald-50 text-emerald-700'
      : status === 'reserved'
        ? 'bg-amber-50 text-amber-700'
        : status === 'delivered'
          ? 'bg-sky-50 text-sky-700'
          : 'bg-slate-100 text-slate-600';

  return <span className={`inline-flex rounded-md px-2 py-1 text-xs font-medium ${className}`}>{productStatusText(status)}</span>;
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

function LogStatusBadge({ success }: { success: boolean }) {
  const className = success ? 'bg-emerald-50 text-emerald-700' : 'bg-red-50 text-red-700';
  return <span className={`inline-flex rounded-md px-2 py-1 text-xs font-medium ${className}`}>{success ? '成功' : '失败'}</span>;
}

function inventoryFilterParams(filters: InventoryFilters) {
  return {
    product_info_id: filters.productInfoId || undefined,
    status: filters.status || undefined,
  };
}

/**
 * 将批量输入转换成库存内容列表。
 *
 * 分隔符留空时兼容 Windows、Linux 和旧式 Mac 的换行符；填写分隔符后按字面文本切分，
 * 避免把用户输入当成正则表达式而产生转义问题。每条内容仅清理首尾空白，内部换行会被保留。
 */
function splitInventoryContents(content: string, separator: string) {
  return content
    .split(separator === '' ? /\r\n|\n|\r/ : separator)
    .map((item) => item.trim())
    .filter(Boolean);
}

function mergeProductOptions(products: Product[], infos: AdminProductInfo[]): ProductOption[] {
  const options = new Map<string, ProductOption>();

  for (const product of products) {
    options.set(product.id, {
      id: product.id,
      image_base64: product.image_base64,
      name: product.name,
      details: product.details ?? '',
      price_cents: product.price_cents,
      sold_count: product.sold_count,
      stock: product.stock,
      active: true,
    });
  }

  for (const info of infos) {
    const existing = options.get(info.id);
    options.set(info.id, {
      id: info.id,
      image_base64: info.image_base64,
      name: info.name,
      details: info.details ?? '',
      price_cents: info.price_cents,
      sold_count: info.sold_count,
      stock: existing?.stock ?? null,
      active: info.active,
    });
  }

  return Array.from(options.values()).sort((left, right) => left.name.localeCompare(right.name, 'zh-CN'));
}

function mergeInventoryProductOptions(productOptions: ProductOption[], inventoryProducts: AdminInventoryProduct[]): ProductOption[] {
  const options = new Map<string, ProductOption>();

  for (const product of productOptions) {
    options.set(product.id, product);
  }

  for (const product of inventoryProducts) {
    if (!options.has(product.product_info_id)) {
      options.set(product.product_info_id, {
        id: product.product_info_id,
        image_base64: null,
        name: product.product_name,
        details: '',
        price_cents: product.price_cents,
        sold_count: 0,
        stock: null,
        active: product.product_info_active,
      });
    }
  }

  return Array.from(options.values()).sort((left, right) => left.name.localeCompare(right.name, 'zh-CN'));
}

async function buildProductInfoPayload(
  form: ProductInfoFormState,
  onError: (message: string | null) => void,
): Promise<CreateProductInfoInput | null> {
  const name = form.name.trim();
  const priceCents = yuanToCents(form.priceYuan);

  if (!name) {
    onError('商品名称不能为空');
    return null;
  }
  if (!Number.isFinite(priceCents) || priceCents < 0) {
    onError('价格格式不正确');
    return null;
  }

  const imageBase64 = form.imageFile ? await readFileAsBase64(form.imageFile, onError) : normalizeOptional(form.imageBase64);
  if (form.imageFile && !imageBase64) {
    return null;
  }

  return {
    image_base64: imageBase64,
    name,
    details: form.details?.trim() ?? '',
    price_cents: priceCents,
    active: form.active,
  };
}

async function safeBuildProductInfoPayload(
  form: ProductInfoFormState,
  onError: (message: string | null) => void,
): Promise<CreateProductInfoInput | null> {
  try {
    return await buildProductInfoPayload(form, onError);
  } catch (err) {
    onError(err instanceof Error ? err.message : '商品信息表单处理失败');
    return null;
  }
}

function revokePreviewUrl(form: ProductInfoFormState) {
  if (form.imagePreviewUrl) {
    URL.revokeObjectURL(form.imagePreviewUrl);
  }
}

function readFileAsBase64(file: File, onError: (message: string | null) => void): Promise<string | null> {
  return new Promise((resolve) => {
    const reader = new FileReader();

    reader.onerror = () => {
      onError('图片读取失败');
      resolve(null);
    };

    reader.onload = () => {
      if (typeof reader.result !== 'string') {
        onError('图片读取失败');
        resolve(null);
        return;
      }

      resolve(reader.result.split(',', 2)[1] ?? reader.result);
    };

    reader.readAsDataURL(file);
  });
}

function normalizeOptional(value: string) {
  const trimmed = value.trim();
  return trimmed ? trimmed : null;
}

function yuanToCents(value: string) {
  return Math.round(Number(value) * 100);
}

function formatPrice(priceCents: number) {
  return currencyFormatter.format(priceCents / 100);
}

function formatDate(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString('zh-CN', { hour12: false });
}

function formatJson(value: unknown) {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function imageBase64Src(imageBase64: string) {
  return imageBase64.startsWith('data:') ? imageBase64 : `data:image/png;base64,${imageBase64}`;
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

function paymentMethodText(provider: string, channel: string) {
  const methods: Record<string, string> = {
    'epay/alipay': '支付宝（易支付）',
    'epay/wxpay': '微信（易支付）',
    'wechatpay/native': '微信支付官方 Native',
  };
  return methods[`${provider}/${channel}`] ?? `${provider}/${channel}`;
}

function productStatusText(status: string) {
  const texts: Record<string, string> = {
    available: '可售',
    reserved: '已预留',
    delivered: '已发货',
    disabled: '禁用',
  };
  return texts[status] ?? status;
}
