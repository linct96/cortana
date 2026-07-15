export type AgentsProfile = {
  id: string;
  name: string;
  content: string;
  isActive: boolean;
};

export type AgentsStatus = {
  profiles: AgentsProfile[];
  path: string;
  fileState: 'managed' | 'unmanaged' | 'missing';
};
