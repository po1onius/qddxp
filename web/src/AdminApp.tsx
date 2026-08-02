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
  PackagePlus,
  Power,
  PowerOff,
  Plus,
  RefreshCcw,
  Save,
  ShieldCheck,
  Store,
  TicketPlus,
  X,
} from 'lucide-react';
import { useToast } from './Toast';
import {
  createAdminProduct,
  createProductInfo,
  getAdminOrderAllocationMode,
  listAdminApiCallLogs,
  listAdminOrders,
  listAdminProductInfo,
  listAdminProducts,
  listProducts,
  updateAdminOrderAllocationMode,
  updateAdminProductStatuses,
  updateProductInfoActive,
} from './api/client';
import type {
  AdminApiCallLog,
  AdminInventoryProduct,
  AdminOrder,
  AdminProductInfo,
  AdminProductStatus,
  CreateProductInfoInput,
  CreateAdminProductResult,
  OrderAllocationMode,
  Product,
  ProductInventoryStatus,
} from './types';

const ADMIN_KEY_STORAGE = 'qddxp_admin_key';
const ADMIN_PAGE_SIZE = 20;
const PRODUCT_INFO_PAGE_SIZE = 8;

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

type AdminTab = 'product_info' | 'inventory' | 'orders' | 'logs';

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

export function AdminApp() {
  const { showToast } = useToast();
  const [adminKey, setAdminKey] = useState(() => localStorage.getItem(ADMIN_KEY_STORAGE) ?? '');
  const [products, setProducts] = useState<Product[]>([]);
  const [adminProductInfos, setAdminProductInfos] = useState<AdminProductInfo[]>([]);
  const [inventoryProducts, setInventoryProducts] = useState<AdminInventoryProduct[]>([]);
  const [inventoryFilters, setInventoryFilters] = useState<InventoryFilters>(emptyInventoryFilters);
  const [orders, setOrders] = useState<AdminOrder[]>([]);
  const [logs, setLogs] = useState<AdminApiCallLog[]>([]);
  const [orderAllocationMode, setOrderAllocationMode] = useState<OrderAllocationMode>('reserve_on_create');
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
  const [loadingOrderAllocationMode, setLoadingOrderAllocationMode] = useState(false);
  const [updatingOrderAllocationMode, setUpdatingOrderAllocationMode] = useState(false);

  const productOptions = useMemo(() => mergeProductOptions(products, adminProductInfos), [products, adminProductInfos]);
  const inventoryProductOptions = useMemo(
    () => mergeInventoryProductOptions(productOptions, inventoryProducts),
    [inventoryProducts, productOptions],
  );
  const totalStock = useMemo(() => products.reduce((sum, product) => sum + product.stock, 0), [products]);
  const totalSold = useMemo(() => products.reduce((sum, product) => sum + product.sold_count, 0), [products]);
  const paidOrders = useMemo(() => orders.filter((order) => order.status === 'paid' || order.status === 'preorder').length, [orders]);
  const successfulLogs = useMemo(() => logs.filter((log) => log.success).length, [logs]);

  useEffect(() => {
    void refreshProducts();
    if (adminKey.trim()) {
      void refreshOrderAllocationMode(adminKey);
      void refreshProductInfo(adminKey);
      void refreshOrders(adminKey);
    }
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

  async function refreshProductInfo(key = adminKey) {
    const trimmedKey = key.trim();
    if (!trimmedKey) {
      return;
    }

    setLoadingProductInfos(true);

    try {
      setAdminProductInfos(await listAdminProductInfo(trimmedKey));
    } catch (err) {
      showToast({
        message: err instanceof Error ? err.message : '商品信息列表加载失败',
        type: 'error',
      });
    } finally {
      setLoadingProductInfos(false);
    }
  }

  async function refreshOrderAllocationMode(key = adminKey) {
    const trimmedKey = key.trim();
    if (!trimmedKey) {
      return;
    }

    setLoadingOrderAllocationMode(true);

    try {
      const response = await getAdminOrderAllocationMode(trimmedKey);
      setOrderAllocationMode(response.order_allocation_mode);
    } catch (err) {
      showToast({
        message: err instanceof Error ? err.message : '下单方式加载失败',
        type: 'error',
      });
    } finally {
      setLoadingOrderAllocationMode(false);
    }
  }

  async function changeOrderAllocationMode(mode: OrderAllocationMode) {
    const trimmedKey = adminKey.trim();
    if (!trimmedKey) {
      showError('请先保存管理员密钥');
      return;
    }
    if (mode === orderAllocationMode) {
      return;
    }

    setUpdatingOrderAllocationMode(true);

    try {
      const response = await updateAdminOrderAllocationMode(trimmedKey, {
        order_allocation_mode: mode,
      });
      setOrderAllocationMode(response.order_allocation_mode);
      showToast({ message: '下单方式已更新', type: 'success' });
    } catch (err) {
      showToast({
        message: err instanceof Error ? err.message : '下单方式更新失败',
        type: 'error',
      });
    } finally {
      setUpdatingOrderAllocationMode(false);
    }
  }

  async function refreshInventory(key = adminKey, filters = inventoryFilters, page = inventoryPage) {
    await loadInventoryPage({ filters, key, page });
  }

  async function loadInventoryPage({
    errorMessage = '库存列表加载失败',
    filters = inventoryFilters,
    key = adminKey,
    page,
  }: {
    errorMessage?: string;
    filters?: InventoryFilters;
    key?: string;
    page: number;
  }) {
    const trimmedKey = key.trim();
    if (!trimmedKey) {
      showError('请先保存管理员密钥');
      return;
    }

    setLoadingInventory(true);

    try {
      const response = await listAdminProducts(trimmedKey, {
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

  async function refreshOrders(key = adminKey, page = ordersPage) {
    await loadOrdersPage({ key, page });
  }

  async function loadOrdersPage({
    errorMessage = '订单列表加载失败',
    key = adminKey,
    page,
  }: {
    errorMessage?: string;
    key?: string;
    page: number;
  }) {
    const trimmedKey = key.trim();
    if (!trimmedKey) {
      showError('请先保存管理员密钥');
      return;
    }

    setLoadingOrders(true);

    try {
      const response = await listAdminOrders(trimmedKey, { page, page_size: ADMIN_PAGE_SIZE });
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

  async function refreshLogs(key = adminKey, page = logsPage) {
    await loadLogsPage({ key, page });
  }

  async function loadLogsPage({
    errorMessage = '日志列表加载失败',
    key = adminKey,
    page,
  }: {
    errorMessage?: string;
    key?: string;
    page: number;
  }) {
    const trimmedKey = key.trim();
    if (!trimmedKey) {
      showError('请先保存管理员密钥');
      return;
    }

    setLoadingLogs(true);

    try {
      const response = await listAdminApiCallLogs(trimmedKey, { page, page_size: ADMIN_PAGE_SIZE });
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

  function saveAdminKey(value: string) {
    const trimmedKey = value.trim();
    if (!trimmedKey) {
      showError('管理员密钥不能为空');
      return;
    }

    localStorage.setItem(ADMIN_KEY_STORAGE, trimmedKey);
    setAdminKey(trimmedKey);
    showToast({ message: '管理员密钥已保存', type: 'success' });
    void refreshOrderAllocationMode(trimmedKey);
    void refreshProductInfo(trimmedKey);
    void refreshOrders(trimmedKey, 1);
    if (activeTab === 'inventory') {
      void refreshInventory(trimmedKey, inventoryFilters, 1);
    }
    if (activeTab === 'logs') {
      void refreshLogs(trimmedKey, 1);
    }
  }

  function clearAdminKey() {
    localStorage.removeItem(ADMIN_KEY_STORAGE);
    setAdminKey('');
    setAdminProductInfos([]);
    setInventoryProducts([]);
    setOrders([]);
    setLogs([]);
    setOrderAllocationMode('reserve_on_create');
    setInventoryPage(1);
    setInventoryTotal(0);
    setOrdersPage(1);
    setOrdersTotal(0);
    setLogsPage(1);
    setLogsTotal(0);
    showToast({ message: '管理员密钥已清除', type: 'success' });
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
    if (adminKey.trim()) {
      void refreshInventory(adminKey, filters, 1);
    }
  }

  function changeTab(tab: AdminTab) {
    setActiveTab(tab);
    if (tab === 'inventory' && adminKey.trim()) {
      void refreshInventory();
    }
    if (tab === 'orders' && adminKey.trim()) {
      void refreshOrders();
    }
    if (tab === 'logs' && adminKey.trim()) {
      void refreshLogs();
    }
  }

  return (
    <div className="min-h-screen bg-zinc-50 text-slate-950">
      <header className="border-b border-slate-200 bg-white">
        <div className="mx-auto flex max-w-7xl flex-col gap-4 px-4 py-4 lg:flex-row lg:items-center lg:justify-between lg:px-8">
          <div>
            <p className="text-sm font-medium text-slate-500">虚拟商品商城</p>
            <h1 className="text-2xl font-semibold tracking-normal">管理后台</h1>
          </div>
          <div className="flex flex-col gap-3 sm:flex-row sm:items-end lg:justify-end">
            <AdminKeyPanel adminKey={adminKey} onClear={clearAdminKey} onSave={saveAdminKey} />
            <OrderAllocationModePanel
              disabled={!adminKey.trim() || loadingOrderAllocationMode || updatingOrderAllocationMode}
              loading={loadingOrderAllocationMode || updatingOrderAllocationMode}
              mode={orderAllocationMode}
              onChange={(mode) => void changeOrderAllocationMode(mode)}
            />
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
        <section className="grid gap-4 md:grid-cols-4">
          <MetricCard icon={<Boxes size={19} />} label="商品种类" value={products.length.toString()} />
          <MetricCard icon={<PackagePlus size={19} />} label="可售库存" value={totalStock.toString()} />
          <MetricCard icon={<TicketPlus size={19} />} label="已售数量" value={totalSold.toString()} />
          <MetricCard
            icon={activeTab === 'logs' ? <FileText size={19} /> : <ClipboardList size={19} />}
            label={activeTab === 'logs' ? '成功日志' : '已支付订单'}
            value={activeTab === 'logs' ? `${successfulLogs}/${logs.length}` : `${paidOrders}/${orders.length}`}
          />
        </section>

        <AdminNav activeTab={activeTab} onChange={changeTab} />

        {activeTab === 'product_info' && (
          <ProductInfoCatalogPanel
            adminKey={adminKey}
            loading={loadingProducts || loadingProductInfos}
            onCreated={(info) => {
              upsertProductInfo(info);
              showToast({ message: `已创建商品信息：${info.name}`, type: 'success' });
              void refreshProducts();
            }}
            onError={showError}
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
              adminKey={adminKey}
              filters={inventoryFilters}
              loading={loadingInventory}
              onFiltersChange={changeInventoryFilters}
              onInventoryCreated={(result) => {
                const assignedText = result.assigned_preorders > 0 ? `已履约 ${result.assigned_preorders} 个预购订单` : '';
                const stockedText = result.stocked > 0 ? `新增 ${result.stocked} 条可售库存` : '';
                showToast({
                  message: [assignedText, stockedText].filter(Boolean).join('，') || '没有新增库存',
                  type: 'success',
                });
                void refreshProducts();
                void refreshInventory(adminKey, inventoryFilters, 1);
                void refreshOrders(adminKey, ordersPage);
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
                void refreshInventory(adminKey, inventoryFilters, inventoryPage);
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
      </main>
    </div>
  );
}

function AdminNav({ activeTab, onChange }: { activeTab: AdminTab; onChange: (tab: AdminTab) => void }) {
  const tabs: Array<{ icon: ReactNode; id: AdminTab; label: string }> = [
    { icon: <Boxes size={18} />, id: 'product_info', label: '商品信息' },
    { icon: <PackagePlus size={18} />, id: 'inventory', label: '库存' },
    { icon: <ClipboardList size={18} />, id: 'orders', label: '订单' },
    { icon: <FileText size={18} />, id: 'logs', label: '日志' },
  ];

  return (
    <nav className="mt-6 flex flex-wrap gap-2 border-b border-slate-200" aria-label="管理导航">
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

function AdminKeyPanel({
  adminKey,
  onClear,
  onSave,
}: {
  adminKey: string;
  onClear: () => void;
  onSave: (value: string) => void;
}) {
  const [value, setValue] = useState(adminKey);
  const [showKey, setShowKey] = useState(false);

  useEffect(() => {
    setValue(adminKey);
  }, [adminKey]);

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    onSave(value);
  }

  return (
    <form className="w-full sm:w-[440px]" onSubmit={submit}>
      <label className="block">
        <div className="flex">
          <input
            className="h-10 min-w-0 flex-1 rounded-l-md border border-slate-300 px-3 text-sm outline-none focus:border-slate-950"
            onChange={(event) => setValue(event.target.value)}
            type={showKey ? 'text' : 'password'}
            value={value}
          />
          <button
            className="inline-flex h-10 w-10 items-center justify-center border-y border-r border-slate-300 bg-white text-slate-600 hover:text-slate-950"
            onClick={() => setShowKey((current) => !current)}
            type="button"
          >
            {showKey ? <EyeOff size={18} /> : <Eye size={18} />}
          </button>
          <button
            className="inline-flex h-10 items-center gap-2 border-y border-r border-slate-300 bg-slate-950 px-3 text-sm font-medium text-white"
            type="submit"
          >
            <ShieldCheck size={17} />
            保存
          </button>
          <button
            className="h-10 rounded-r-md border-y border-r border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:text-slate-950"
            onClick={onClear}
            type="button"
          >
            清除
          </button>
        </div>
      </label>
    </form>
  );
}

function OrderAllocationModePanel({
  disabled,
  loading,
  mode,
  onChange,
}: {
  disabled: boolean;
  loading: boolean;
  mode: OrderAllocationMode;
  onChange: (mode: OrderAllocationMode) => void;
}) {
  const options: Array<{ label: string; mode: OrderAllocationMode }> = [
    { label: '创建锁定', mode: 'reserve_on_create' },
    { label: '支付分配', mode: 'allocate_on_pay' },
  ];

  return (
    <div className="w-full sm:w-auto">
      <div className="mb-1 flex items-center gap-2 text-xs font-medium text-slate-500">
        <span>下单方式</span>
        {loading && <Loader2 className="animate-spin" size={14} />}
      </div>
      <div className="inline-flex h-10 w-full overflow-hidden rounded-md border border-slate-300 bg-white sm:w-auto">
        {options.map((option) => (
          <button
            aria-pressed={mode === option.mode}
            className={`min-w-0 flex-1 px-3 text-sm font-medium sm:flex-none ${
              mode === option.mode ? 'bg-slate-950 text-white' : 'text-slate-700 hover:bg-slate-50'
            } disabled:cursor-not-allowed disabled:opacity-60`}
            disabled={disabled}
            key={option.mode}
            onClick={() => onChange(option.mode)}
            type="button"
          >
            {option.label}
          </button>
        ))}
      </div>
    </div>
  );
}

function ProductInfoCatalogPanel({
  adminKey,
  loading,
  onCreated,
  onError,
  onStatusChanged,
  productOptions,
}: {
  adminKey: string;
  loading: boolean;
  onCreated: (info: AdminProductInfo) => void;
  onError: (message: string | null) => void;
  onStatusChanged: (info: AdminProductInfo) => void;
  productOptions: ProductOption[];
}) {
  const [showCreateModal, setShowCreateModal] = useState(false);
  const [updatingProductInfoId, setUpdatingProductInfoId] = useState<string | null>(null);
  const [page, setPage] = useState(1);
  const totalPages = Math.max(1, Math.ceil(productOptions.length / PRODUCT_INFO_PAGE_SIZE));
  const visibleProductOptions = productOptions.slice((page - 1) * PRODUCT_INFO_PAGE_SIZE, page * PRODUCT_INFO_PAGE_SIZE);

  useEffect(() => {
    if (page > totalPages) {
      setPage(totalPages);
    }
  }, [page, totalPages]);

  async function toggleProductActive(product: ProductOption) {
    if (!adminKey.trim()) {
      onError('请先保存管理员密钥');
      return;
    }

    const active = !product.active;
    setUpdatingProductInfoId(product.id);
    onError(null);

    try {
      const info = await updateProductInfoActive(adminKey.trim(), product.id, { active });
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
          {loading && <Loader2 className="animate-spin text-slate-500" size={18} />}
          {productOptions.length > PRODUCT_INFO_PAGE_SIZE && (
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
            onToggleActive={() => void toggleProductActive(product)}
            product={product}
            updating={updatingProductInfoId === product.id}
          />
        ))}
      </div>

      {productOptions.length === 0 && !loading && (
        <p className="mt-4 rounded-md border border-slate-200 bg-white px-3 py-2 text-sm text-slate-500">暂无商品信息</p>
      )}

      {showCreateModal && (
        <ProductInfoModal
          adminKey={adminKey}
          onClose={() => setShowCreateModal(false)}
          onCreated={(info) => {
            setPage(1);
            onCreated(info);
          }}
          onError={onError}
        />
      )}
    </section>
  );
}

function ProductInfoCard({
  onToggleActive,
  product,
  updating,
}: {
  onToggleActive: () => void;
  product: ProductOption;
  updating: boolean;
}) {
  const imageSrc = product.image_base64 ? imageBase64Src(product.image_base64) : null;
  const actionText = product.active ? '下架' : '上架';

  return (
    <article className="overflow-hidden rounded-md border border-slate-200 bg-white text-left shadow-panel">
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
          <StatusPill active={product.active} />
          <button
            aria-label={`${actionText}商品信息 ${product.name}`}
            className="inline-flex h-9 items-center justify-center gap-2 rounded-md border border-slate-300 bg-white px-3 text-sm font-medium text-slate-700 hover:border-slate-500 disabled:cursor-wait disabled:opacity-60"
            disabled={updating}
            onClick={onToggleActive}
            type="button"
          >
            {updating ? <Loader2 className="animate-spin" size={17} /> : product.active ? <PowerOff size={17} /> : <Power size={17} />}
            {actionText}
          </button>
        </div>
      </div>
    </article>
  );
}

function ProductInfoModal({
  adminKey,
  onClose,
  onCreated,
  onError,
}: {
  adminKey: string;
  onClose: () => void;
  onCreated: (info: AdminProductInfo) => void;
  onError: (message: string | null) => void;
}) {
  const [form, setForm] = useState(emptyProductInfoForm);
  const [submitting, setSubmitting] = useState(false);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!adminKey.trim()) {
      onError('请先保存管理员密钥');
      return;
    }

    const payload = await safeBuildProductInfoPayload(form, onError);
    if (!payload) {
      return;
    }

    setSubmitting(true);
    onError(null);

    try {
      const info = await createProductInfo(adminKey.trim(), payload);
      onCreated(info);
      revokePreviewUrl(form);
      onClose();
    } catch (err) {
      onError(err instanceof Error ? err.message : '创建商品信息失败');
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="fixed inset-0 z-40 flex items-end justify-center bg-slate-950/45 px-4 py-4 sm:items-center">
      <section className="max-h-[calc(100vh-2rem)] w-full max-w-3xl overflow-hidden rounded-md bg-white shadow-panel">
        <div className="flex items-start justify-between gap-4 border-b border-slate-200 px-5 py-4">
          <div>
            <h2 className="text-lg font-semibold">新增商品信息</h2>
            <p className="mt-1 text-sm text-slate-500">填写商品基础信息和展示内容</p>
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
          <ProductInfoFields form={form} onChange={setForm} />
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
              创建
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
}: {
  form: ProductInfoFormState;
  onChange: (form: ProductInfoFormState) => void;
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
      <label className="mt-4 flex items-center gap-2 text-sm font-medium text-slate-700">
        <input
          checked={form.active}
          className="h-4 w-4 rounded border-slate-300"
          onChange={(event) => onChange({ ...form, active: event.target.checked })}
          type="checkbox"
        />
        上架
      </label>
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
  adminKey,
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
  adminKey: string;
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
    if (!adminKey.trim()) {
      onError('请先保存管理员密钥');
      return;
    }

    setUpdatingStatus(status);
    onError(null);

    try {
      const result = await updateAdminProductStatuses(adminKey.trim(), {
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
          adminKey={adminKey}
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
  adminKey,
  initialProductInfoId,
  onClose,
  onCreated,
  onError,
  productOptions,
}: {
  adminKey: string;
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

    if (!adminKey.trim()) {
      onError('请先保存管理员密钥');
      return;
    }
    if (!productInfoId) {
      onError('请选择商品信息');
      return;
    }
    if (contents.length === 0) {
      onError('库存内容不能为空');
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
      const createdProducts = await createAdminProduct(adminKey.trim(), {
        product_info_id: productInfoId,
        contents,
      });
      console.info('[库存补货] 批量库存提交成功', {
        productInfoId,
        createdCount: createdProducts.items.length,
        assignedPreorders: createdProducts.assigned_preorders,
        stocked: createdProducts.stocked,
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
              className={textareaClass}
              onChange={(event) => setContent(event.target.value)}
              placeholder="输入多条发货内容"
              value={content}
            />
          </label>
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
  orders,
  page,
  pageSize,
  total,
}: {
  loading: boolean;
  onNextPage: () => void;
  onPreviousPage: () => void;
  onRefresh: () => void;
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
        <table className="min-w-[920px] text-left text-sm">
          <thead className="border-b border-slate-200 text-xs uppercase text-slate-500">
            <tr>
              <th className="py-2 pr-3 font-medium">订单号</th>
              <th className="px-3 py-2 font-medium">商品</th>
              <th className="px-3 py-2 font-medium">联系方式</th>
              <th className="px-3 py-2 font-medium">状态</th>
              <th className="px-3 py-2 font-medium">下单时间</th>
              <th className="px-3 py-2 font-medium">发货内容</th>
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
                  <OrderStatusBadge status={order.status} />
                </td>
                <td className="px-3 py-2 text-slate-600">{formatDate(order.created_at)}</td>
                <td className="max-w-[260px] px-3 py-2">
                  <pre className="max-h-20 overflow-auto whitespace-pre-wrap break-words rounded-md bg-slate-50 p-2 text-xs text-slate-700">
                    {order.product_content ?? '待补货'}
                  </pre>
                </td>
              </tr>
            ))}
            {orders.length === 0 && (
              <tr>
                <td className="py-6 text-sm text-slate-500" colSpan={6}>
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

function MetricCard({ icon, label, value }: { icon: ReactNode; label: string; value: string }) {
  return (
    <div className="rounded-md border border-slate-200 bg-white p-4 shadow-panel">
      <div className="flex items-center justify-between gap-3">
        <p className="text-sm text-slate-500">{label}</p>
        <span className="text-slate-500">{icon}</span>
      </div>
      <p className="mt-3 text-2xl font-semibold">{value}</p>
    </div>
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

function OrderStatusBadge({ status }: { status: string }) {
  const paid = status === 'paid';
  const pending = status === 'pending';
  const preorder = status === 'preorder';
  const className = paid
    ? 'bg-emerald-50 text-emerald-700'
    : pending
      ? 'bg-amber-50 text-amber-700'
      : preorder
        ? 'bg-sky-50 text-sky-700'
        : 'bg-slate-100 text-slate-600';

  return <span className={`inline-flex rounded-md px-2 py-1 text-xs font-medium ${className}`}>{statusText(status)}</span>;
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

function statusText(status: string) {
  const texts: Record<string, string> = {
    pending: '待支付',
    paid: '已支付',
    preorder: '预购',
  };
  return texts[status] ?? status;
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
