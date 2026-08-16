import type {
  AdminApiCallLog,
  AdminInventoryProduct,
  AdminOrder,
  AdminProductInfo,
  CreateAdminProductInput,
  CreateAdminProductResult,
  CreateOrderInput,
  CreateOrderResult,
  CreateProductInfoInput,
  ListOrdersByContactInput,
  OffsetPageResponse,
  PaymentMethod,
  StorefrontConfig,
  OrderDetail,
  OrderSummary,
  Product,
  QueryOrderInput,
  UpdateAdminProductStatusResult,
  UpdateAdminProductStatusInput,
  UpdateProductInfoInput,
  UpdateProductInfoActiveInput,
  UpdateOrderRemarkInput,
  UpdateOrderRemarkResult,
} from '../types';

const DEFAULT_API_BASE_URL = '/api';
const configuredApiBaseUrl = (import.meta.env.VITE_API_BASE_URL ?? '').trim();
const API_BASE_URL = (configuredApiBaseUrl || DEFAULT_API_BASE_URL).replace(/\/+$/, '');
export const ADMIN_SESSION_EXPIRED_EVENT = 'qddxp:admin-session-expired';

export class ApiError extends Error {
  readonly status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

function apiUrl(path: string): string {
  const normalizedPath = path.startsWith('/') ? path : `/${path}`;
  return `${API_BASE_URL}${normalizedPath}`;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(apiUrl(path), {
    ...init,
    // 生产环境和 Vite 开发代理均为同源请求。显式声明凭据策略，确保管理员的
    // HttpOnly 会话 Cookie 会随管理 API 请求发送，同时不会泄露给跨域地址。
    credentials: 'same-origin',
    headers: {
      'Content-Type': 'application/json',
      ...init?.headers,
    },
  });

  if (!response.ok) {
    const body = await response.json().catch(() => ({}));
    throw new ApiError(body.error ?? `请求失败: ${response.status}`, response.status);
  }

  if (response.status === 204) {
    return undefined as T;
  }

  return response.json() as Promise<T>;
}

function withQuery(path: string, params: Record<string, string | number | undefined>) {
  const searchParams = new URLSearchParams();
  Object.entries(params).forEach(([key, value]) => {
    if (value !== undefined) {
      searchParams.set(key, String(value));
    }
  });
  const query = searchParams.toString();
  return query ? `${path}?${query}` : path;
}

export async function getStorefrontConfig(): Promise<StorefrontConfig> {
  const config = await request<StorefrontConfig>('/storefront');
  // Logo 与配置接口来自同一 API 服务。使用统一的 API 基础地址，兼容开发代理和独立 API 域名。
  return { ...config, logo_url: apiUrl('/storefront/logo') };
}

export type ProductPageParams = {
  page?: number;
  page_size?: number;
};

export type AdminProductPageParams = ProductPageParams & {
  product_info_id?: string;
  status?: string;
};

export function listProductPage(params: ProductPageParams = {}): Promise<OffsetPageResponse<Product>> {
  return request<OffsetPageResponse<Product>>(withQuery('/products', params));
}

export async function listProducts(): Promise<Product[]> {
  const page = await listProductPage({ page_size: 100 });
  return page.items;
}

export function getProduct(productId: string): Promise<Product> {
  return request<Product>(`/products/${encodeURIComponent(productId)}`);
}

export function listPaymentMethods(): Promise<PaymentMethod[]> {
  return request<PaymentMethod[]>('/payment-methods');
}

export function createOrder(input: CreateOrderInput): Promise<CreateOrderResult> {
  return request<CreateOrderResult>('/orders', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export function queryOrder(input: QueryOrderInput): Promise<OrderDetail> {
  return request<OrderDetail>('/orders/query', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export function listOrdersByContact(input: ListOrdersByContactInput): Promise<OffsetPageResponse<OrderSummary>> {
  return request<OffsetPageResponse<OrderSummary>>('/orders/by-contact', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export type AdminSessionStatus = {
  authenticated: boolean;
};

export function getAdminSession(): Promise<AdminSessionStatus> {
  return request<AdminSessionStatus>('/admin/session');
}

export function loginAdmin(adminKey: string): Promise<AdminSessionStatus> {
  return request<AdminSessionStatus>('/admin/session', {
    method: 'POST',
    body: JSON.stringify({ admin_key: adminKey }),
  });
}

export function logoutAdmin(): Promise<void> {
  return request<void>('/admin/session', { method: 'DELETE' });
}

async function adminRequest<T>(path: string, init?: RequestInit): Promise<T> {
  try {
    return await request<T>(path, init);
  } catch (error) {
    if (error instanceof ApiError && error.status === 401) {
      // 会话可能在页面停留期间因空闲而过期。统一发布事件，使管理页立即卸载敏感
      // 数据并返回登录表单，避免每个业务组件各自维护一套过期处理分支。
      window.dispatchEvent(new Event(ADMIN_SESSION_EXPIRED_EVENT));
      throw new ApiError('管理员会话已失效，请重新登录', 401);
    }
    throw error;
  }
}

export function createProductInfo(input: CreateProductInfoInput): Promise<AdminProductInfo> {
  return adminRequest<AdminProductInfo>('/admin/product-info', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export function listAdminProductInfo(): Promise<AdminProductInfo[]> {
  return adminRequest<AdminProductInfo[]>('/admin/product-info');
}

export function updateProductInfo(
  productInfoId: string,
  input: UpdateProductInfoInput,
): Promise<AdminProductInfo> {
  return adminRequest<AdminProductInfo>(`/admin/product-info/${productInfoId}`, {
    method: 'PATCH',
    body: JSON.stringify(input),
  });
}

export function updateProductInfoActive(
  productInfoId: string,
  input: UpdateProductInfoActiveInput,
): Promise<AdminProductInfo> {
  return adminRequest<AdminProductInfo>(`/admin/product-info/${productInfoId}/active`, {
    method: 'PATCH',
    body: JSON.stringify(input),
  });
}

export function createAdminProduct(input: CreateAdminProductInput): Promise<CreateAdminProductResult> {
  return adminRequest<CreateAdminProductResult>('/admin/products', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export function updateAdminProductStatuses(
  input: UpdateAdminProductStatusInput,
): Promise<UpdateAdminProductStatusResult> {
  return adminRequest<UpdateAdminProductStatusResult>('/admin/products/status', {
    method: 'PATCH',
    body: JSON.stringify(input),
  });
}

export function listAdminProducts(
  params: AdminProductPageParams = {},
): Promise<OffsetPageResponse<AdminInventoryProduct>> {
  return adminRequest<OffsetPageResponse<AdminInventoryProduct>>(
    withQuery('/admin/products', {
      page: params.page,
      page_size: params.page_size,
      product_info_id: params.product_info_id,
      status: params.status,
    }),
  );
}

export function listAdminOrders(params: ProductPageParams = {}): Promise<OffsetPageResponse<AdminOrder>> {
  return adminRequest<OffsetPageResponse<AdminOrder>>(withQuery('/admin/orders', params));
}

export function updateAdminOrderRemark(
  orderId: string,
  input: UpdateOrderRemarkInput,
): Promise<UpdateOrderRemarkResult> {
  return adminRequest<UpdateOrderRemarkResult>(`/admin/orders/${encodeURIComponent(orderId)}/remark`, {
    method: 'PATCH',
    body: JSON.stringify(input),
  });
}

export function listAdminApiCallLogs(
  params: ProductPageParams = {},
): Promise<OffsetPageResponse<AdminApiCallLog>> {
  return adminRequest<OffsetPageResponse<AdminApiCallLog>>(withQuery('/admin/api-call-logs', params));
}
