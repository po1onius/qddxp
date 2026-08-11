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
  ReconcilePaymentResult,
  OrderDetail,
  OrderSummary,
  Product,
  QueryOrderInput,
  UpdateAdminProductStatusResult,
  UpdateAdminProductStatusInput,
  UpdateProductInfoActiveInput,
} from '../types';

const DEFAULT_API_BASE_URL = '/api';
const configuredApiBaseUrl = (import.meta.env.VITE_API_BASE_URL ?? '').trim();
const API_BASE_URL = (configuredApiBaseUrl || DEFAULT_API_BASE_URL).replace(/\/+$/, '');

function apiUrl(path: string): string {
  const normalizedPath = path.startsWith('/') ? path : `/${path}`;
  return `${API_BASE_URL}${normalizedPath}`;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(apiUrl(path), {
    ...init,
    headers: {
      'Content-Type': 'application/json',
      ...init?.headers,
    },
  });

  if (!response.ok) {
    const body = await response.json().catch(() => ({}));
    throw new Error(body.error ?? `请求失败: ${response.status}`);
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

export function reconcileWechatPayOrder(
  orderId: string,
  orderPassword: string,
): Promise<ReconcilePaymentResult> {
  return request<ReconcilePaymentResult>(`/orders/${orderId}/payments/wechatpay/query`, {
    method: 'POST',
    body: JSON.stringify({ order_password: orderPassword }),
  });
}

export function listOrdersByContact(input: ListOrdersByContactInput): Promise<OffsetPageResponse<OrderSummary>> {
  return request<OffsetPageResponse<OrderSummary>>('/orders/by-contact', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

function adminHeaders(adminKey: string) {
  return {
    'x-admin-key': adminKey,
  };
}

export function createProductInfo(adminKey: string, input: CreateProductInfoInput): Promise<AdminProductInfo> {
  return request<AdminProductInfo>('/admin/product-info', {
    method: 'POST',
    headers: adminHeaders(adminKey),
    body: JSON.stringify(input),
  });
}

export function listAdminProductInfo(adminKey: string): Promise<AdminProductInfo[]> {
  return request<AdminProductInfo[]>('/admin/product-info', {
    headers: adminHeaders(adminKey),
  });
}

export function updateProductInfoActive(
  adminKey: string,
  productInfoId: string,
  input: UpdateProductInfoActiveInput,
): Promise<AdminProductInfo> {
  return request<AdminProductInfo>(`/admin/product-info/${productInfoId}/active`, {
    method: 'PATCH',
    headers: adminHeaders(adminKey),
    body: JSON.stringify(input),
  });
}

export function createAdminProduct(adminKey: string, input: CreateAdminProductInput): Promise<CreateAdminProductResult> {
  return request<CreateAdminProductResult>('/admin/products', {
    method: 'POST',
    headers: adminHeaders(adminKey),
    body: JSON.stringify(input),
  });
}

export function updateAdminProductStatuses(
  adminKey: string,
  input: UpdateAdminProductStatusInput,
): Promise<UpdateAdminProductStatusResult> {
  return request<UpdateAdminProductStatusResult>('/admin/products/status', {
    method: 'PATCH',
    headers: adminHeaders(adminKey),
    body: JSON.stringify(input),
  });
}

export function listAdminProducts(
  adminKey: string,
  params: AdminProductPageParams = {},
): Promise<OffsetPageResponse<AdminInventoryProduct>> {
  return request<OffsetPageResponse<AdminInventoryProduct>>(
    withQuery('/admin/products', {
      page: params.page,
      page_size: params.page_size,
      product_info_id: params.product_info_id,
      status: params.status,
    }),
    {
      headers: adminHeaders(adminKey),
    },
  );
}

export function listAdminOrders(adminKey: string, params: ProductPageParams = {}): Promise<OffsetPageResponse<AdminOrder>> {
  return request<OffsetPageResponse<AdminOrder>>(withQuery('/admin/orders', params), {
    headers: adminHeaders(adminKey),
  });
}

export function listAdminApiCallLogs(
  adminKey: string,
  params: ProductPageParams = {},
): Promise<OffsetPageResponse<AdminApiCallLog>> {
  return request<OffsetPageResponse<AdminApiCallLog>>(withQuery('/admin/api-call-logs', params), {
    headers: adminHeaders(adminKey),
  });
}
