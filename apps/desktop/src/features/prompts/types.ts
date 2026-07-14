export type AgentsProfile = {
  id: string;
  name: string;
  content: string;
  isActive: boolean;
};

export type AgentsStatus = {
  profiles: AgentsProfile[];
  activeProfileId: string | null;
  path: string;
  fileState: 'managed' | 'external' | 'unmanaged' | 'missing';
};
