CREATE TABLE app_metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE TABLE app_commands(sequence INTEGER PRIMARY KEY,key TEXT NOT NULL UNIQUE,operation TEXT NOT NULL,outcome TEXT,rejection TEXT,effect_state TEXT);
CREATE TABLE app_observations(sequence INTEGER PRIMARY KEY,command_key TEXT NOT NULL,outcome TEXT NOT NULL,UNIQUE(command_key,outcome));
CREATE TABLE app_events(sequence INTEGER PRIMARY KEY,id TEXT NOT NULL UNIQUE,event TEXT NOT NULL,result TEXT);
CREATE TABLE app_business(key TEXT PRIMARY KEY,kind TEXT NOT NULL,input_digest TEXT NOT NULL,task_id TEXT NOT NULL,conversation_id TEXT NOT NULL,source_event TEXT NOT NULL,payload TEXT NOT NULL);
PRAGMA user_version=1;
