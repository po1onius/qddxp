export type StorefrontConfig = {
  shop_name: string;
  logo_url: string;
};

export type Product = {
  id: string;
  image_base64: string | null;
  name: string;
  details: string;
  price_cents: number;
  sold_count: number;
  stock: number;
};

export type OffsetPageResponse<T> = {
  items: T[];
  page: number;
  page_size: number;
  total: number;
};

export type AdminProductInfo = {
  id: string;
  image_base64: string | null;
  name: string;
  details: string;
  price_cents: number;
  sold_count: number;
  active: boolean;
  created_at: string;
};

export type CreateProductInfoInput = {
  image_base64: string | null;
  name: string;
  details: string;
  price_cents: number;
  active: boolean;
};

export type UpdateProductInfoActiveInput = {
  active: boolean;
};

export type AdminProductStatus = 'available' | 'disabled';

export type ProductInventoryStatus = 'available' | 'reserved' | 'delivered' | 'disabled';

export type CreateAdminProductInput = {
  product_info_id: string;
  contents: string[];
};

export type CreateAdminProductResult = {
  items: AdminProduct[];
  stocked: number;
};

export type UpdateAdminProductStatusInput = {
  product_ids: string[];
  status: AdminProductStatus;
};

export type UpdateAdminProductStatusResult = {
  selected: number;
  updated: number;
  ignored: number;
  status: AdminProductStatus | string;
};

export type AdminProduct = {
  id: string;
  product_info_id: string;
  content: string;
  status: AdminProductStatus | string;
  created_at: string;
};

export type AdminInventoryProduct = {
  id: string;
  product_info_id: string;
  product_name: string;
  price_cents: number;
  product_info_active: boolean;
  content: string;
  status: ProductInventoryStatus | string;
  created_at: string;
};

export type CreateOrderInput = {
  product_info_id: string;
  contact: string;
  order_password: string;
  payment: PaymentSelection;
};

export type PaymentProvider = 'epay' | 'wechatpay';
export type PaymentChannel = 'alipay' | 'wxpay' | 'native';

export type PaymentSelection = {
  provider: PaymentProvider;
  channel: PaymentChannel;
};

export type PaymentMethod = PaymentSelection & {
  label: string;
  action_type: 'redirect' | 'qr_code';
};

export type PaymentAction =
  | { type: 'redirect'; url: string }
  | { type: 'qr_code'; content: string; expires_at: string };

export type CreateOrderResult = {
  id: string;
  status: OrderStatus;
  payment_action: PaymentAction | null;
  payment_error: string | null;
};

export type QueryOrderInput = {
  id: string;
  order_password: string;
};

export type ListOrdersByContactInput = {
  contact: string;
  page?: number;
  page_size?: number;
};

export type OrderStatus = 'pending' | 'paid' | 'expired' | 'cancelled';

export type ReconcilePaymentResult = {
  status: OrderStatus;
  trade_state: string;
};

export type OrderSummary = {
  id: string;
  product_info_id: string;
  product_name: string;
  price_cents: number;
  status: OrderStatus;
  paid_at: string | null;
  created_at: string;
};

export type OrderDetail = {
  id: string;
  product_info_id: string;
  product_name: string;
  status: OrderStatus;
  paid_at: string | null;
  contact: string;
  created_at: string;
  content: string | null;
};

export type AdminOrder = {
  id: string;
  product_id: string | null;
  product_info_id: string;
  product_name: string;
  product_content: string | null;
  created_at: string;
  paid_at: string | null;
  status: OrderStatus | string;
  contact: string;
  payment_provider: string;
  payment_channel: string;
  merchant_trade_no: string;
  provider_transaction_id: string | null;
  payment_state: string;
  amount_cents: number;
  currency: string;
};

export type AdminApiCallLog = {
  id: string;
  api_name: string;
  http_method: string;
  path: string;
  request_params: Record<string, unknown>;
  response_status: number;
  response_body: string;
  success: boolean;
  error_message: string | null;
  created_at: string;
};
